"""class-methods-all · 列出类名空间下所有符号（不仅限 vtable）"""
from . import common


def run(program, args):
    if not args:
        print("用法: ./run.sh class-methods-all <ClassName>")
        return
    class_name = args[0]

    st = program.getSymbolTable()
    global_ns = program.getGlobalNamespace()
    ns = st.getNamespace(class_name, global_ns)
    if ns is None:
        print(f"未找到类 {class_name}")
        return

    # List all symbols in this namespace
    print(f"类 {class_name} 的所有符号:")
    from ghidra.program.model.symbol import SymbolType
    syms = list(st.getSymbols(ns))
    print(f"  命名空间直接符号: {len(syms)}")
    for s in syms:
        t = s.getSymbolType()
        print(f"    [{t}] {s.getName()}  @ 0x{s.getAddress()}")

    # Search for functions/labels that might reference this class in xrefs
    print(f"\n扫描全局函数中 parent namespace 是 {class_name}:")
    fm = program.getFunctionManager()
    count = 0
    for func in fm.getFunctions(True):
        parent = func.getParentNamespace()
        if parent and parent.getName() == class_name:
            count += 1
            if count <= 50:
                print(f"    {common.func_signature_brief(func)}")
    print(f"  共 {count}")

    # Search xref of any vtable/type descriptor — callers
    print(f"\n扫描到该类 vtable 的 xref（前 20 个）:")
    vftable_addr = None
    for sym in syms:
        if sym.getName() == "vftable":
            vftable_addr = sym.getAddress()
            break
    if vftable_addr:
        rm = program.getReferenceManager()
        refs = rm.getReferencesTo(vftable_addr)
        n = 0
        for ref in refs:
            if n >= 20: break
            from_addr = ref.getFromAddress()
            from_func = fm.getFunctionContaining(from_addr)
            if from_func:
                print(f"    from 0x{from_addr}  ({from_func.getName()} @ 0x{from_func.getEntryPoint()})")
            else:
                print(f"    from 0x{from_addr}  (no function)")
            n += 1
