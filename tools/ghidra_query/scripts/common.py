"""共享工具：打开 project / program，拿 decompiler。"""
import os
from pathlib import Path


def open_project(readonly: bool = True):
    """打开 Ghidra project（默认只读）"""
    from ghidra.base.project import GhidraProject
    project_dir = os.environ.get("GHIDRA_PROJECT_DIR",
                                 str(Path.home() / "Projects/Ghidra-Projects"))
    project_name = os.environ.get("GHIDRA_PROJECT_NAME", "FlexColor-RE")
    return GhidraProject.openProject(project_dir, project_name, readonly)


def open_program(project, program_name: str, readonly: bool = True):
    """从 project 打开指定 program（默认只读）"""
    return project.openProgram("/", program_name, readonly)


def get_decompiler(program):
    """构造 DecompInterface，返回 (iface, monitor)"""
    from ghidra.app.decompiler import DecompInterface
    from ghidra.util.task import ConsoleTaskMonitor
    iface = DecompInterface()
    iface.openProgram(program)
    monitor = ConsoleTaskMonitor()
    return iface, monitor


def decompile_function(iface, monitor, func, timeout_sec: int = 30) -> str:
    """反编译单个函数，返回 C 源文本；失败返回错误说明"""
    result = iface.decompileFunction(func, timeout_sec, monitor)
    if result.decompileCompleted():
        return str(result.getDecompiledFunction().getC())
    return f"// decompile failed: {result.getErrorMessage()}"


def iter_functions_by_class_name(program, class_name: str):
    """按类名找所有函数。Ghidra 的 RTTI 分析器恢复了类命名空间但没把虚方法挂回去，
    所以我们通过 **遍历 vftable** 来列出虚方法。"""
    # 先尝试直接 parent namespace（少见但可能存在）
    fm = program.getFunctionManager()
    direct = []
    for func in fm.getFunctions(True):
        parent = func.getParentNamespace()
        if parent and parent.getName() == class_name:
            direct.append(func)
    if direct:
        yield from direct
        return

    # 否则走 vftable 路径
    yield from iter_functions_by_vftable(program, class_name)


def iter_functions_by_vftable(program, class_name: str):
    """遍历类的 vftable，返回每一个 slot 指向的 function"""
    st = program.getSymbolTable()
    global_ns = program.getGlobalNamespace()
    ns = st.getNamespace(class_name, global_ns)
    if not ns:
        return

    # 找 vftable 符号
    vftable_addr = None
    for sym in st.getSymbols(ns):
        if sym.getName() == "vftable":
            vftable_addr = sym.getAddress()
            break
    if vftable_addr is None:
        return

    memory = program.getMemory()
    fm = program.getFunctionManager()
    pointer_size = program.getDefaultPointerSize()
    af = program.getAddressFactory()

    addr = vftable_addr
    for _ in range(200):  # sanity cap
        try:
            if pointer_size == 4:
                val = memory.getInt(addr) & 0xFFFFFFFF
            else:
                val = memory.getLong(addr) & 0xFFFFFFFFFFFFFFFF
        except Exception:
            break
        target = af.getAddress(hex(val)[2:])
        if target is None:
            break
        func = fm.getFunctionAt(target)
        if func is None:
            # End of vftable (hit data or non-function address)
            break
        yield func
        addr = addr.add(pointer_size)


def iter_functions_by_name_prefix(program, prefix: str):
    """按函数名前缀匹配（例如 "StretchNegGamma"）"""
    fm = program.getFunctionManager()
    for func in fm.getFunctions(True):
        if func.getName().startswith(prefix):
            yield func


def symbol_table(program):
    return program.getSymbolTable()


def data_type_manager(program):
    return program.getDataTypeManager()


def func_signature_brief(func) -> str:
    """返回函数的简短签名，用于列表显示"""
    sig = func.getSignature()
    addr = func.getEntryPoint()
    return f"{sig} @ 0x{addr}"
