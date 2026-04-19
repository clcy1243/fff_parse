"""dump-pipeline · 把所有色彩 pipeline 类的方法反编译导出到 Markdown"""
import os
from pathlib import Path
from . import common

# 默认关心的类集合（可通过 args 覆盖）
DEFAULT_CLASSES = [
    # 色彩空间基础设施
    "CColorWorld",
    "CCachedColorWorld",
    "CColorManager",
    "CICMColorManager",
    "CColorCorrection",
    "CColorTempConversion",
    # 曲线对象
    "CCurve",
    "CAggregateCurve",
    "CBufferedCurve",
    "CFilmCurve",
    "CGammaCurve",
    "CGammaNegCurve",
    "CContrastCurve",
    "CContrastBrightnessCurve",
    "CHighShadowCurve",
    # Pipeline 入口
    "CImageCorrection",
]

# 独立函数（非类方法）
DEFAULT_FUNCTIONS = [
    "StretchNegGamma",
    "TranslateColors",  # Win32 ICM wrapper 可能
]


def run(program, args):
    # Parse args: output file, optional class list
    out_path = "/tmp/flexcolor_pipeline.md"
    classes = list(DEFAULT_CLASSES)
    functions = list(DEFAULT_FUNCTIONS)

    i = 0
    while i < len(args):
        if args[i] == "--out" and i + 1 < len(args):
            out_path = args[i + 1]
            i += 2
        elif args[i] == "--class" and i + 1 < len(args):
            classes = [args[i + 1]]  # override list
            i += 2
        else:
            i += 1

    iface, monitor = common.get_decompiler(program)
    try:
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(f"# FlexColor 色彩 Pipeline 反编译 dump\n\n")
            f.write(f"Program: `{program.getName()}`  \n")
            f.write(f"Architecture: `{program.getLanguage().getLanguageID()}`  \n\n")
            f.write("---\n\n")

            # 类方法
            for cls in classes:
                funcs = list(common.iter_functions_by_class_name(program, cls))
                if not funcs:
                    continue
                f.write(f"## class `{cls}` ({len(funcs)} 方法)\n\n")
                # Sort by entry point for stable order
                funcs.sort(key=lambda x: str(x.getEntryPoint()))
                for func in funcs:
                    sig = func.getSignature()
                    addr = func.getEntryPoint()
                    f.write(f"### `{func.getName()}` @ 0x{addr}\n\n")
                    f.write(f"**Signature**: `{sig}`\n\n")
                    f.write("```c\n")
                    code = common.decompile_function(iface, monitor, func, timeout_sec=60)
                    f.write(code)
                    f.write("\n```\n\n")
                f.write("\n---\n\n")
                print(f"  ✓ {cls} ({len(funcs)} 方法)")

            # 独立函数
            f.write("## 独立函数\n\n")
            for fn_name in functions:
                funcs = list(common.iter_functions_by_name_prefix(program, fn_name))
                if not funcs:
                    continue
                for func in funcs:
                    sig = func.getSignature()
                    addr = func.getEntryPoint()
                    f.write(f"### `{func.getName()}` @ 0x{addr}\n\n")
                    f.write(f"**Signature**: `{sig}`\n\n")
                    f.write("```c\n")
                    code = common.decompile_function(iface, monitor, func, timeout_sec=60)
                    f.write(code)
                    f.write("\n```\n\n")
                print(f"  ✓ {fn_name}")

        size_kb = os.path.getsize(out_path) // 1024
        print(f"\n→ 导出 {out_path} ({size_kb} KB)")
    finally:
        iface.dispose()
