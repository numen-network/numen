#!/usr/bin/env bash
#
# Drive a weight sweep from the dev machine. Builds the artefacts here,
# sweeps them on the reference machine and unpacks the result in place.
# Pass pallet names to narrow the run, STEPS and REPEAT to override the
# sweep defaults.
#
# The reference machine answers to the ssh entry named by $HOST and grants
# passwordless sudo.
set -euo pipefail

cd "$(dirname "$0")/.."

HOST=${HOST:-bench}
SSH_OPTS=(-o ServerAliveInterval=30 -o ServerAliveCountMax=6)

BINARY=target/production/numen
RUNTIME=target/production/wbuild/numen-runtime/numen_runtime.compact.compressed.wasm

cargo build --profile production --locked -p numen --features runtime-benchmarks

# Cloud images name themselves after their private address, which says
# nothing about the role and pins every weight header to one instance.
ssh "${SSH_OPTS[@]}" "$HOST" bash -s <<'EOF'
set -euo pipefail
old=$(hostname)
if [[ $old != numen-bench ]]; then
	sudo hostnamectl set-hostname numen-bench
	sudo sed -i "s/$old/numen-bench/g" /etc/hosts
fi
mkdir -p numen-bench
EOF

# The sweep rewrites the weight files it is asked for, then rebuilds the
# module list from whatever sits in the directory. Current files travel
# along so a narrowed run leaves the rest of that list standing.
tar czf - .maintain runtime/src/weights pallets/*/src/weights.rs |
	ssh "${SSH_OPTS[@]}" "$HOST" 'tar xzf - -C numen-bench'
scp "${SSH_OPTS[@]}" "$BINARY" "$RUNTIME" "$HOST:numen-bench/"

ssh "${SSH_OPTS[@]}" "$HOST" bash -s <<EOF
set -euo pipefail
cd numen-bench
chmod +x numen
${STEPS:+STEPS=$STEPS} ${REPEAT:+REPEAT=$REPEAT} \
	BINARY=\$PWD/numen \
	RUNTIME=\$PWD/numen_runtime.compact.compressed.wasm \
	.maintain/run-benchmarks.sh $*
EOF

ssh "${SSH_OPTS[@]}" "$HOST" 'cd numen-bench && tar czf - pallets/*/src/weights.rs runtime/src/weights' | tar xzf -
