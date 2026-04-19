# ghidra_query

通过 `pyghidra` 从 Python 脚本查询 Ghidra 已分析的 FlexColor.dll。

## 前置条件

1. **Ghidra 12.x**（brew 安装）
   ```bash
   brew install ghidra
   ```
   依赖 openjdk@21，brew 会自动装。

2. **Ghidra project 已导入并跑过 auto-analysis**
   - 默认路径：`/Users/will/Projects/Ghidra-Projects/FlexColor-RE`
   - 导入的 binary：`FlexColor.dll` + `HasDeviceLink64.dll`
   - 首次打开 GUI 导入 + 分析（一次性 ~50 分钟），之后用脚本即可

3. **Python 3.10–3.13 + pyghidra 独立 venv**
   ```bash
   python3.10 -m venv ~/.venvs/pyghidra
   source ~/.venvs/pyghidra/bin/activate
   pip install pyghidra
   ```
   （Python 3.14 与 jpype 不兼容，**必须用 3.13 或以下**）

## 使用

确保 **Ghidra GUI 已关闭**（它会锁 project）。然后：

```bash
cd tools/ghidra_query
./run.sh list-classes                     # 所有 C++ 类名
./run.sh list-methods CFilmCurve          # 某类的所有方法
./run.sh decompile "CContrastCurve::Apply"# 单函数反编译
./run.sh dump-pipeline                    # 全量 dump 色彩 pipeline 类
./run.sh find-luts                        # 扫 .rdata 找 LUT 数据
./run.sh vtable CFilmCurve                # 看 vtable 布局
```

## 文件结构

```
tools/ghidra_query/
├── README.md              # 本文件
├── run.sh                 # 包装脚本（设置 env + 激活 venv + 调 python）
├── ghidra_query.py        # 主入口，子命令 dispatch
└── scripts/               # 具体查询逻辑（单一职责，容易扩展）
    ├── __init__.py
    ├── common.py          # 共享：open_program, get_decompiler, etc.
    ├── list_classes.py
    ├── list_methods.py
    ├── decompile.py
    ├── dump_pipeline.py
    ├── find_luts.py
    └── vtable.py
```

## 扩展

加新查询类型：
1. 在 `scripts/` 加 `your_query.py`，导出 `run(program, args)` 函数
2. 在 `ghidra_query.py` 注册到 `COMMANDS` dict
3. 更新本 README

## 环境变量覆盖

```bash
export GHIDRA_PROJECT_DIR=/path/to/Ghidra-Projects     # 默认: ~/Projects/Ghidra-Projects
export GHIDRA_PROJECT_NAME=MyProject                   # 默认: FlexColor-RE
export GHIDRA_PROGRAM=FlexColor.dll                    # 默认: FlexColor.dll
export PYGHIDRA_VENV=~/.venvs/pyghidra                 # 默认
export GHIDRA_INSTALL_DIR=/usr/local/Cellar/ghidra/12.0.4/libexec
export JAVA_HOME=/usr/local/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home
```
