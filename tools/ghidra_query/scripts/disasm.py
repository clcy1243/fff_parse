"""disasm · 反汇编指定地址的函数（或前 N 条指令）"""


def run(program, args):
    if not args:
        print("用法: ./run.sh disasm 0xADDR [maxInstr=200]")
        return
    addr_str = args[0]
    if addr_str.startswith("0x") or addr_str.startswith("0X"):
        addr_str = addr_str[2:]
    max_instr = int(args[1]) if len(args) > 1 else 200

    af = program.getAddressFactory()
    listing = program.getListing()
    start = af.getAddress(addr_str)
    if start is None:
        print(f"无效地址: {args[0]}")
        return

    func = program.getFunctionManager().getFunctionAt(start)
    if func:
        print(f"// {func.getSignature()}")
        body = func.getBody()
        end = body.getMaxAddress()
    else:
        print(f"// (地址不在函数起点，从该地址开始)")
        end = None

    count = 0
    instr = listing.getInstructionAt(start)
    while instr and count < max_instr:
        mnem = instr.getMnemonicString()
        # Operands with their representations
        ops = []
        for i in range(instr.getNumOperands()):
            ops.append(instr.getDefaultOperandRepresentation(i))
        code_units = ", ".join(ops)

        # Any references on this instruction (data refs inline)
        refs_info = ""
        for ref in instr.getReferencesFrom():
            to = ref.getToAddress()
            if ref.getReferenceType().isData():
                data = listing.getDataAt(to)
                if data and data.isDefined():
                    val = data.getValue()
                    refs_info += f"  [{to} = {val!r}]"

        print(f"  0x{instr.getAddress()}  {mnem:<8} {code_units}{refs_info}")
        next_addr = instr.getAddress().add(instr.getLength())
        if end and next_addr > end:
            break
        instr = listing.getInstructionAt(next_addr)
        count += 1
    if instr is None or (end and instr and instr.getAddress() > end):
        print(f"\n// 结束 ({count} 条指令)")
    elif count >= max_instr:
        print(f"\n// 已达 maxInstr 限制 {max_instr}")
