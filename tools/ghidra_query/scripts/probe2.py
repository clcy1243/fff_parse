"""list all analyzers that ran + data types for RTTI"""
def run(program, args):
    # List all data types
    dtm = program.getDataTypeManager()
    print(f"Data types: {dtm.getDataTypeCount(False)}")
    rtti_count = 0
    for dt in dtm.getAllDataTypes():
        name = dt.getName()
        if "RTTI" in name or "vftable" in name.lower():
            if rtti_count < 20:
                print(f"  {name}  ({dt.getCategoryPath()})")
            rtti_count += 1
    print(f"\nRTTI-related types: {rtti_count}")
    
    # Check options for analyzers
    from ghidra.app.services import AnalysisPriority
    print("\nChecking analysis status...")
    opts = program.getOptions("Analyzers")
    for name in opts.getOptionNames():
        if "RTTI" in name or "rtti" in name.lower() or "Windows" in name or "class" in name.lower():
            val = opts.getBoolean(name, False) if opts.getType(name).toString() == "BOOLEAN_TYPE" else "?"
            print(f"  {name}: {val}")
