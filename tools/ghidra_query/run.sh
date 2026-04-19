#!/usr/bin/env bash
# ghidra_query 包装：设置 Ghidra / Java / pyghidra venv 环境，然后转发给 ghidra_query.py
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export GHIDRA_INSTALL_DIR="${GHIDRA_INSTALL_DIR:-/usr/local/Cellar/ghidra/12.0.4/libexec}"
export JAVA_HOME="${JAVA_HOME:-/usr/local/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home}"
PYGHIDRA_VENV="${PYGHIDRA_VENV:-$HOME/.venvs/pyghidra}"

if [[ ! -d "$PYGHIDRA_VENV" ]]; then
    echo "⚠️  pyghidra venv 未找到：$PYGHIDRA_VENV"
    echo "按 README 先建 venv："
    echo "    python3.10 -m venv $PYGHIDRA_VENV"
    echo "    source $PYGHIDRA_VENV/bin/activate"
    echo "    pip install pyghidra"
    exit 1
fi

# shellcheck disable=SC1091
source "$PYGHIDRA_VENV/bin/activate"

exec python3 "$SCRIPT_DIR/ghidra_query.py" "$@"
