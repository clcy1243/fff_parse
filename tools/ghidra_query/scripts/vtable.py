"""vtable · 打印指定类的 vtable（虚方法列表）和构造器可见的成员字段"""
from . import common


def run(program, args):
    if not args:
        print("用法: ./run.sh vtable <ClassName>")
        return
    class_name = args[0]

    # Find vftable symbol: "ClassName::vftable" (prefer exact "vftable" over "vftable_meta_ptr")
    st = common.symbol_table(program)
    vftable_addr = None
    meta_addr = None
    for sym in st.getAllSymbols(False):
        name = sym.getName()
        parent = sym.getParentNamespace()
        if parent and parent.getName() == class_name:
            if name == "vftable":
                vftable_addr = sym.getAddress()
                break
            elif "vftable_meta" in name:
                meta_addr = sym.getAddress()
    if vftable_addr is None and meta_addr is not None:
        # meta_ptr 一般是 vftable - 4，往前挪 4 字节（32-bit）
        try:
            vftable_addr = meta_addr.add(4)
            print(f"(使用 meta_ptr+4 作为 vftable)")
        except Exception:
            pass
    if vftable_addr:
        print(f"找到 vtable @ 0x{vftable_addr}")

    if vftable_addr is None:
        print(f"未找到 {class_name} 的 vtable。尝试列所有 {class_name} 的符号...")
        for sym in st.getAllSymbols(False):
            parent = sym.getParentNamespace()
            if parent and parent.getName() == class_name:
                print(f"  {sym.getName()} @ 0x{sym.getAddress()}")
        return

    # Walk vtable: read pointers until we hit a non-code pointer
    memory = program.getMemory()
    fm = program.getFunctionManager()
    pointer_size = program.getDefaultPointerSize()

    print(f"\nvtable slots (pointer_size = {pointer_size}):")
    addr = vftable_addr
    for i in range(100):  # sanity cap
        try:
            if pointer_size == 4:
                val = memory.getInt(addr) & 0xFFFFFFFF
            else:
                val = memory.getLong(addr) & 0xFFFFFFFFFFFFFFFF
        except Exception:
            break

        target = program.getAddressFactory().getAddress(hex(val)[2:])
        if target is None:
            break
        func = fm.getFunctionAt(target)
        if func is None:
            # Not a function pointer — end of vtable
            break

        sig = func.getSignature()
        parent = func.getParentNamespace()
        parent_name = parent.getName() if parent else ""
        mark = "" if parent_name == class_name else f"  (inherited from {parent_name})"
        print(f"  [{i:>3}] 0x{target}  {func.getName()}{mark}")
        addr = addr.add(pointer_size)
