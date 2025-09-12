#!/usr/bin/env bash
set -euo pipefail

target_dir=$(cargo metadata --format-version=1 | python3 -c "import sys, json; print(json.load(sys.stdin)['target_directory'])")
echo "target_dir=$target_dir"

# out_dir="$target_dir/xtensa-esp32s3-espidf/debug/build/esp-idf-sys-e50adf61d0fbb40d/out"
out_dir=$(find "$target_dir"/xtensa-esp32s3-espidf/*/build/esp-idf-sys-*/out -maxdepth 0 -type d 2>/dev/null | head -n1 || true)
if [ ! -d "$out_dir" ]; then
  echo "Error: Could not locate esp-idf-sys build out directory." >&2
  exit 1
fi

echo "out_dir=$out_dir"

cp -r $out_dir/sdkconfig target/
cp -r sdkconfig $out_dir

esp_sr_path="$out_dir/managed_components/espressif__esp-sr"
sdkconfig_path="$out_dir/sdkconfig"
build_path="."

mkdir -p "$build_path"

python3 "$esp_sr_path/model/movemodel.py" -d1 "$sdkconfig_path" -d2 "$esp_sr_path" -d3 "$build_path"

