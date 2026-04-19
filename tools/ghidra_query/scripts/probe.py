"""probe - scan all symbols for pattern"""
def run(program, args):
    import re
    pattern = re.compile(args[0] if args else "FilmCurve", re.I)
    st = program.getSymbolTable()
    matches = 0
    for sym in st.getAllSymbols(False):
        name = sym.getName()
        if pattern.search(name):
            parent = sym.getParentNamespace()
            parent_name = parent.getName() if parent else ""
            sym_type = sym.getSymbolType()
            print(f"  {name}  [{sym_type}]  parent={parent_name}")
            matches += 1
            if matches >= 30:
                print(f"  ... (truncated, showing first 30)")
                break
    print(f"\n共 {matches} 个匹配")
