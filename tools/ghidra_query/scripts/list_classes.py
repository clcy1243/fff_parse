"""list-classes · 列出所有 C++ 类（按 RTTI 恢复的 namespace）"""
import re


def run(program, args):
    pattern_str = args[0] if args else None
    pattern = re.compile(pattern_str, re.IGNORECASE) if pattern_str else None

    # Collect class namespaces from functions
    classes = {}
    fm = program.getFunctionManager()
    for func in fm.getFunctions(True):
        parent = func.getParentNamespace()
        if parent is None:
            continue
        name = parent.getName()
        if not name or name == "<global>":
            continue
        # Filter by pattern if provided
        if pattern and not pattern.search(name):
            continue
        classes.setdefault(name, 0)
        classes[name] += 1

    # Print sorted by name
    print(f"{'Class':<50} {'#methods':>8}")
    print("-" * 60)
    for name in sorted(classes.keys()):
        count = classes[name]
        print(f"{name:<50} {count:>8}")
    print(f"\n共 {len(classes)} 个类")
