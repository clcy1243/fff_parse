"""probe3 - comprehensive search"""
def run(program, args):
    name_target = args[0] if args else "CFilmCurve"
    st = program.getSymbolTable()
    
    # Approach 1: getSymbols by name
    print(f"Approach 1 — getSymbols('{name_target}'):")
    syms = list(st.getSymbols(name_target))
    print(f"  Found {len(syms)}")
    for sym in syms[:5]:
        print(f"    {sym.getName(True)}  [{sym.getSymbolType()}]  @ 0x{sym.getAddress()}")

    # Approach 2: namespace-based
    print(f"\nApproach 2 — getNamespace():")
    from ghidra.program.model.symbol import Namespace
    globalNs = program.getGlobalNamespace()
    ns = st.getNamespace(name_target, globalNs)
    if ns:
        print(f"  Got namespace: {ns}")
        # List symbols in it
        children = list(st.getSymbols(ns))
        print(f"  Children: {len(children)}")
        for s in children[:15]:
            print(f"    {s.getName()}  [{s.getSymbolType()}]")
    else:
        print(f"  No namespace '{name_target}'")

    # Approach 3: all classes
    print(f"\nApproach 3 — program.getListing().getDefinedData() filter:")
    print(f"  Skip (too slow)")
    
    # Approach 4: classes in class manager
    print(f"\nApproach 4 — use SymbolIterator looking for CLASS_NS:")
    from ghidra.program.model.symbol import SymbolType
    count = 0
    match = None
    for sym in st.getAllSymbols(False):
        if sym.getSymbolType() == SymbolType.CLASS and name_target in sym.getName():
            match = sym
            print(f"  Class sym: {sym.getName(True)}  @ 0x{sym.getAddress()}")
            count += 1
            if count >= 5:
                break
    if not match:
        print(f"  No CLASS symbol contains '{name_target}'")
