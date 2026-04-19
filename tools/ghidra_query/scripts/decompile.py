"""decompile · 反编译指定函数（支持 ClassName::Method、address 0x...、函数名）"""
from . import common


def run(program, args):
    if not args:
        print("用法: ./run.sh decompile <ClassName::Method | 0xADDR | FunctionName>")
        return
    target = args[0]

    func = _resolve(program, target)
    if func is None:
        print(f"未找到: {target}")
        return

    iface, monitor = common.get_decompiler(program)
    try:
        code = common.decompile_function(iface, monitor, func)
        print(f"// {common.func_signature_brief(func)}")
        print(code)
    finally:
        iface.dispose()


def _resolve(program, target: str):
    fm = program.getFunctionManager()

    # Case 1: hex address
    if target.startswith("0x") or target.startswith("0X"):
        from ghidra.program.model.address import AddressFactory
        af = program.getAddressFactory()
        addr = af.getAddress(target[2:])
        if addr:
            return fm.getFunctionAt(addr)

    # Case 2: ClassName::Method
    if "::" in target:
        cls, _, meth = target.rpartition("::")
        for func in common.iter_functions_by_class_name(program, cls):
            if func.getName() == meth:
                return func
        # Try matching just the method name loosely
        for func in common.iter_functions_by_class_name(program, cls):
            if meth in func.getName():
                return func
        return None

    # Case 3: bare function name (first match)
    for func in fm.getFunctions(True):
        if func.getName() == target:
            return func
    # Loose match
    for func in fm.getFunctions(True):
        if target in func.getName():
            return func
    return None
