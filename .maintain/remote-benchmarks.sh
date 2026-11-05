#!/usr/bin/env bash
#
# Drive a weight sweep from the dev machine. Builds the artefacts here,
# sweeps them on the reference machine and unpacks the result in place.
# Pass pallet names to narrow the run, STEPS and REPEAT to override the
# sweep defaults. JITTER sets the percent drift a remeasured file must
# reach to replace the committed one.
#
# The reference machine answers to the ssh entry named by $HOST.
set -euo pipefail

cd "$(dirname "$0")/.."

HOST=${HOST:-bench}
SSH_OPTS=(-o ServerAliveInterval=30 -o ServerAliveCountMax=6)

BINARY=target/production/numen
RUNTIME=target/production/wbuild/numen-runtime/numen_runtime.compact.compressed.wasm

command -v subweight >/dev/null || {
	echo "missing subweight, run: cargo install subweight --locked" >&2
	exit 1
}

cargo build --profile production --locked -p numen --features runtime-benchmarks

ssh "${SSH_OPTS[@]}" "$HOST" 'mkdir -p numen-bench'

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

# A remeasure on the same CPU moves each extrinsic by a few percent. Files
# that stay inside that band keep their committed version. A new CPU is a
# new reference machine, so every file it measured stays.
JITTER=${JITTER:-5}
committed_dir=$(mktemp -d)
trap 'rm -rf "$committed_dir"' EXIT
git diff --name-only -- 'pallets/*/src/weights.rs' runtime/src/weights ':!runtime/src/weights/mod.rs' |
	while read -r file; do
		# subweight pairs old and new files by name.
		committed=$committed_dir/$(basename "$file")
		git show "HEAD:$file" >"$committed"
		[[ $(grep -m1 -o 'CPU: .*' "$committed") == "$(grep -m1 -o 'CPU: .*' "$file")" ]] || continue
		if report=$(subweight compare files --old "$committed" --new "$file" \
			--method asymptotic --change added changed removed \
			--threshold "$JITTER" --format csv --no-color) &&
			[[ $report == 'No changes found.' ]]; then
			git checkout -- "$file"
		fi
	done
