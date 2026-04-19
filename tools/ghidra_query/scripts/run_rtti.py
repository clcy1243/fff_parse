"""run-rtti · 跑 Windows PE RTTI 分析器恢复 C++ 类结构（需要可写模式）"""


def run(program, args):
    from ghidra.app.plugin.core.analysis import AutoAnalysisManager
    from ghidra.util.task import ConsoleTaskMonitor
    from ghidra.program.model.listing import Program

    # 先列出可用分析器并显示哪些跟 RTTI 相关
    mgr = AutoAnalysisManager.getAnalysisManager(program)
    options = program.getOptions("Analyzers")

    print("扫描与 RTTI/C++ 类相关的分析器:")
    rtti_analyzers = []
    for name in options.getOptionNames():
        if any(kw in name.lower() for kw in ["rtti", "class", "windows", "vftable", "virtual"]):
            try:
                val = options.getBoolean(name, False)
                is_enabled = bool(val)
            except Exception:
                is_enabled = "?"
            print(f"  [{is_enabled}] {name}")
            if "rtti" in name.lower() or "class" in name.lower() or "vftable" in name.lower():
                rtti_analyzers.append(name)

    if not rtti_analyzers:
        print("\n未找到明确的 RTTI 分析器选项。")
        return

    # 强制启用所有 RTTI 相关分析器
    print(f"\n启用 {len(rtti_analyzers)} 个 RTTI 分析器并重新运行 auto-analysis...")
    for name in rtti_analyzers:
        try:
            options.setBoolean(name, True)
            print(f"  ✓ 启用 {name}")
        except Exception as e:
            print(f"  × 启用失败 {name}: {e}")

    monitor = ConsoleTaskMonitor()
    print("\n开始分析（可能需要几分钟）...")

    # 触发完整重分析
    mgr.initializeOptions()
    mgr.reAnalyzeAll(None)
    mgr.startAnalysis(monitor)

    print(f"\n分析完成。当前类命名空间:")

    # 统计恢复到的类
    st = program.getSymbolTable()
    classes = set()
    for sym in st.getAllSymbols(False):
        parent = sym.getParentNamespace()
        if parent is None:
            continue
        name = parent.getName()
        # C++ 类名启发：以 C 开头大写字母、或含 ::
        if name.startswith("C") and len(name) > 2 and name[1].isupper():
            classes.add(name)

    sample = sorted(classes)
    # 先打印我们最关心的
    targets = ["CFilmCurve", "CContrastCurve", "CGammaCurve", "CGammaNegCurve",
               "CAggregateCurve", "CImageCorrection", "CColorWorld", "CColorCorrection"]
    print(f"\n目标类恢复情况:")
    for t in targets:
        mark = "✓" if t in classes else "✗"
        print(f"  [{mark}] {t}")

    print(f"\n共恢复 {len(classes)} 个 C 开头类。前 30 个:")
    for n in sample[:30]:
        print(f"  {n}")
