#!/bin/bash
# Test OSC 8 hyperlinks in terminal

osc8() {
  local url="$1" text="$2"
  printf '\033]8;;%s\033\\%s\033]8;;\033\\' "$url" "$text"
}

echo "Terminal hyperlink test:"
echo ""
printf "  Website: "
osc8 "https://zedra.dev" "zedra.dev"
echo ""
echo ""

printf "  README:  "
osc8 "file://$(cd "$(dirname "$0")/.." && pwd)/README.md" "README.md"
echo ""
echo ""

printf "  crates/zedra/src/app.rs:  "
osc8 "file://$(cd "$(dirname "$0")/.." && pwd)/crates/zedra/src/app.rs" "app.rs"
echo ""
echo ""
echo "Press in links above to test."
