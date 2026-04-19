"""list-methods · 列出指定类的所有方法"""
from . import common


def run(program, args):
    if not args:
        print("用法: ./run.sh list-methods <ClassName>")
        return
    class_name = args[0]
    funcs = list(common.iter_functions_by_class_name(program, class_name))
    if not funcs:
        print(f"未找到类 {class_name}")
        return
    print(f"{class_name} 的方法 ({len(funcs)} 个):")
    for f in sorted(funcs, key=lambda x: str(x.getEntryPoint())):
        print(f"  {common.func_signature_brief(f)}")
