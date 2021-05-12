#!/bin/bash

CONFIG="$1"
SUFFIX="$2"

if [ ! -f "$CONFIG" ]; then
    echo "[-] config file does not exist"
    exit 1
fi

if [ -z "$SUFFIX" ]; then
    echo "[-] suffix required"
    exit 1
fi

. "$CONFIG"

if [ -z "$NET_PREFIX" ]; then
    echo "[-] NET_PREFIX not defined"
    exit 1
fi

if [ -z "$EXTERNAL_IPS" ]; then
    echo "[-] EXTERNAL_IPS not defined"
    exit 1
fi

if [ -z "$IMAGE_PREFIX" ]; then
    echo "[-] IMAGE_PREFIX not defined"
    exit 1
fi

if [ -z "$NAMESPACE" ]; then
    echo "[-] NAMESPACE not defined"
    exit 1
fi

if [ -z "$TIKV_CLUSTER" ]; then
    echo "[-] TIKV_CLUSTER not defined"
    exit 1
fi

# Allow empty suffix
#if [ -z "$IMAGE_SUFFIX" ]; then
#    echo "[-] IMAGE_SUFFIX not defined"
#    exit 1
#fi

rm -r "./k8s.$SUFFIX"
cp -r "`dirname $0`/k8s" "./k8s.$SUFFIX" || exit 1

find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__NET_PREFIX__#$NET_PREFIX#g" '{}' ';'
find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__EXTERNAL_IPS__#$EXTERNAL_IPS#g" '{}' ';'
find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__IMAGE_PREFIX__#$IMAGE_PREFIX#g" '{}' ';'
find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__IMAGE_SUFFIX__#$IMAGE_SUFFIX#g" '{}' ';'
find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__NAMESPACE__#$NAMESPACE#g" '{}' ';'
find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__TIKV_CLUSTER__#$TIKV_CLUSTER#g" '{}' ';'

if [ -z "$IMAGE_PULL_SECRET" ]; then
    find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__MAYBE_PULL_SECRETS__##g" '{}' ';'
else
    find "./k8s.$SUFFIX" -name "*.yaml" -exec sed -i "s#__MAYBE_PULL_SECRETS__#imagePullSecrets:\n      - name: \"$IMAGE_PULL_SECRET\"#g" '{}' ';'
fi

cd "./k8s.$SUFFIX" || exit 1
echo "#!/bin/sh" > apply.sh || exit 1
echo "cd \"\`dirname \$0\`\"" >> apply.sh || exit 1
find . -name "*.yaml" -exec 'echo' 'kubectl apply -f' '{}' ';' >> apply.sh || exit 1
chmod +x apply.sh || exit 1
