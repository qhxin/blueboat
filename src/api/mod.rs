pub mod apns;
pub mod codec;
mod crypto;
pub mod dataset;
pub mod external;
mod fetch;
pub mod graphics;
pub mod kv;
mod mysql;
pub mod task;
pub mod tera;
pub mod text;
pub mod util;
pub mod validation;

use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use itertools::Itertools;
use phf::phf_map;
use std::convert::TryFrom;
use thiserror::Error;
use v8;

use crate::{
  exec::Executor,
  headers::HDR_RES_BUSY_DURATION,
  ipc::{BlueboatIpcRes, BlueboatResponse},
  lpch::{BackgroundEntry, LowPriorityMsg},
  objserde::serialize_v8_value,
  v8util::{create_arraybuffer_from_bytes, FunctionCallbackArgumentsExt},
};

use self::util::{v8_deserialize, write_applog};

pub type ApiHandler = fn(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  retval: v8::ReturnValue,
) -> Result<()>;

pub static API: phf::Map<&'static str, ApiHandler> = phf_map! {
  "sleep" => api_sleep,
  "complete" => api_complete,
  "schedule_at_most_once" => api_schedule_at_most_once,
  "schedule_at_least_once" => task::api_schedule_at_least_once,
  "schedule_delayed" => task::api_schedule_delayed,
  "encode" => api_encode,
  "decode" => api_decode,
  "fetch" => fetch::api_fetch,
  "log" => api_log,
  "crypto_digest" => crypto::api_crypto_digest,
  "crypto_getrandom" => crypto::api_crypto_getrandom,
  "crypto_random_uuid" => crypto::api_crypto_random_uuid,
  "crypto_x25519_derive_public" => crypto::curve25519::api_crypto_x25519_derive_public,
  "crypto_x25519_diffie_hellman" => crypto::curve25519::api_crypto_x25519_diffie_hellman,
  "crypto_ed25519_derive_public" => crypto::curve25519::api_crypto_ed25519_derive_public,
  "crypto_ed25519_sign" => crypto::curve25519::api_crypto_ed25519_sign,
  "crypto_ed25519_verify" => crypto::curve25519::api_crypto_ed25519_verify,
  "crypto_ed25519_pubkey_to_x25519" => crypto::curve25519::api_crypto_ed25519_pubkey_to_x25519,
  "crypto_x25519_pubkey_to_ed25519" => crypto::curve25519::api_crypto_x25519_pubkey_to_ed25519,
  "crypto_jwt_encode" => crypto::jwt::api_crypto_jwt_encode,
  "crypto_jwt_decode" => crypto::jwt::api_crypto_jwt_decode,
  "mysql_exec" => mysql::api_mysql_exec,
  "mysql_start_transaction" => mysql::api_mysql_start_transaction,
  "mysql_end_transaction" => mysql::api_mysql_end_transaction,
  "apns_send" => apns::api_apns_send,
  "codec_hexencode" => codec::api_codec_hexencode,
  "codec_hexdecode" => codec::api_codec_hexdecode,
  "codec_b64encode" => codec::api_codec_b64encode,
  "codec_b64decode" => codec::api_codec_b64decode,
  "codec_multipart_decode" => codec::multipart::api_codec_multipart_decode,
  "graphics_canvas_commit" => graphics::api_graphics_canvas_commit,
  "graphics_canvas_render_svg" => graphics::svg::api_graphics_canvas_render_svg,
  "graphics_canvas_encode" => graphics::codec::api_graphics_canvas_encode,
  "graphics_canvas_draw" => graphics::draw::api_graphics_canvas_draw,
  "graphics_layout_solve" => graphics::layout::api_graphics_layout_solve,
  "graphics_text_measure" => graphics::text::api_graphics_text_measure,
  "tera_render" => tera::api_tera_render,
  "jtd_load_schema" => validation::jtd::api_jtd_load_schema,
  "jtd_validate" => validation::jtd::api_jtd_validate,
  "dataset_mime_guess_by_ext" => dataset::mime::api_dataset_mime_guess_by_ext,
  "text_markdown_render" => text::markdown::api_text_markdown_render,
  "text_yaml_parse" => text::yaml::api_text_yaml_parse,
  "text_yaml_stringify" => text::yaml::api_text_yaml_stringify,
  "external_s3_sign" => external::s3::api_external_s3_sign,
  "external_s3_list_objects_v2" => external::s3::api_external_s3_list_objects_v2,
  "kv_get_many" => kv::api_kv_get_many,
  "kv_compare_and_set_many" => kv::api_kv_compare_and_set_many,
  "kv_prefix_list" => kv::api_kv_prefix_list,
  "kv_prefix_delete" => kv::api_kv_prefix_delete,
};

#[derive(Error, Debug)]
#[error("type mismatch")]
struct TypeMismatch;

#[derive(Error, Debug)]
#[error("serialization error")]
struct SerializationError;

fn api_sleep(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  _retval: v8::ReturnValue,
) -> Result<()> {
  let duration_ms = v8::Local::<v8::Number>::try_from(args.get(1))?
    .uint32_value(scope)
    .ok_or_else(|| TypeMismatch)?;
  let callback = v8::Global::new(scope, args.load_function_at(2)?);
  let exec = Executor::try_current_result()?;
  Executor::spawn(&exec.clone(), async move {
    tokio::time::sleep(Duration::from_millis(duration_ms as u64)).await;
    Executor::enter(&exec, |scope| {
      let callback = v8::Local::new(scope, &callback);
      let undef = v8::undefined(scope);
      callback.call(scope, undef.into(), &[]);
    });
  });

  Ok(())
}

fn api_schedule_at_most_once(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  _retval: v8::ReturnValue,
) -> Result<()> {
  let wire_bytes = serialize_v8_value(scope, args.get(1))?;
  let e = Executor::try_current_result()?.upgrade().unwrap();
  e.ctx
    .lp_tx
    .send(LowPriorityMsg::Background(BackgroundEntry {
      app: e.ctx.key.clone(),
      request_id: e.request_id.clone(),
      wire_bytes,
      same_version: true,
    }))?;
  Ok(())
}

fn api_complete(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  _retval: v8::ReturnValue,
) -> Result<()> {
  let mut res: BlueboatResponse = v8_deserialize(scope, args.get(1))?;

  let body = args.get(2);
  let mut body_bytes = Bytes::new();
  if !body.is_undefined() {
    if let Ok(body) = v8::Local::<v8::Uint8Array>::try_from(body) {
      let mut buf = vec![0u8; body.byte_length()];
      body.copy_contents(&mut buf);
      body_bytes = Bytes::from(buf);
    }
  }

  // Unify header keys and filter out `x-blueboat` headers.
  res.headers = res
    .headers
    .into_iter()
    .map(|(k, v)| (k.to_lowercase(), v))
    .filter(|(k, _)| !k.starts_with("x-blueboat-"))
    .collect();

  res.headers.insert(
    HDR_RES_BUSY_DURATION.into(),
    vec![format!(
      "{:.2}",
      Executor::try_current_result()?
        .upgrade()
        .unwrap()
        .busy_duration
        .get()
        .as_secs_f64()
        * 1000.0
    )],
  );

  Executor::complete(
    &Executor::try_current_result()?,
    BlueboatIpcRes {
      response: res,
      body: body_bytes,
    },
  );
  Ok(())
}

fn api_encode(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  mut retval: v8::ReturnValue,
) -> Result<()> {
  let s = v8::Local::<v8::String>::try_from(args.get(1))?
    .to_rust_string_lossy(scope)
    .into_bytes();
  let buf = create_arraybuffer_from_bytes(scope, &s);
  let view = v8::Uint8Array::new(scope, buf, 0, s.len()).unwrap();
  retval.set(view.into());
  Ok(())
}

fn api_decode(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  mut retval: v8::ReturnValue,
) -> Result<()> {
  let s = v8::Local::<v8::Uint8Array>::try_from(args.get(1))?;
  let mut buf = vec![0u8; s.byte_length()];
  s.copy_contents(&mut buf);
  let s = String::from_utf8_lossy(&buf);
  retval.set(v8::String::new(scope, &s).unwrap().into());
  Ok(())
}

fn api_log(
  scope: &mut v8::HandleScope,
  args: v8::FunctionCallbackArguments,
  _retval: v8::ReturnValue,
) -> Result<()> {
  let message = (1..args.length())
    .map(|i| {
      let arg = v8::Local::new(scope, args.get(i));
      let arg = if let Ok(arg) = v8::Local::<v8::String>::try_from(arg) {
        arg
      } else {
        match v8::json::stringify(scope, arg) {
          Some(x) => x,
          None => v8::String::new(scope, "<norepr>").unwrap(),
        }
      };
      arg.to_rust_string_lossy(scope)
    })
    .join(" ");
  write_applog(scope, message);
  Ok(())
}
