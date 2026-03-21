fn main() {
    // Windows: 将 icon.ico 嵌入到 .exe 文件中，使资源管理器显示自定义图标
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}
