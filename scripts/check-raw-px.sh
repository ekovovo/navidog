#!/usr/bin/env bash
# 裸值检查：Navicat 实测数值只能出现在 navidog-ui 的 token 与主题定义里。
# 例外（运行时测量的几何、平台边界）在该行或上一行写 `// raw-px: <理由>`。
# `.px(...)` 是水平内边距方法，不在检查范围内。
set -euo pipefail
cd "$(dirname "$0")/.."

hits=$(find crates -name '*.rs' ! -path 'crates/navidog-ui/src/tokens.rs' -print0 \
  | xargs -0 awk '
      FNR == 1 { prev = "" }
      /(^|[^.A-Za-z0-9_])(px|rgba?|hsla?)\(/ && $0 !~ /raw-px:/ && prev !~ /raw-px:/ {
        print FILENAME ":" FNR ": " $0
      }
      { prev = $0 }
    ')

if [[ -n "$hits" ]]; then
  echo "发现 token 定义之外的裸像素或颜色值："
  echo "$hits"
  exit 1
fi
echo "裸值检查通过"
