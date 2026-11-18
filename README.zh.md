<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview 图标" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>适用于 Windows 和 Linux 的 Minecraft 投影与结构查看器</strong></p>

[![Platform](https://img.shields.io/badge/Platform-Windows%20%26%20Linux%20x64-0078d4?style=flat-square)](https://github.com/Arcadi4/LitematicaPreview)
[![Tauri 2](https://img.shields.io/badge/Tauri-v2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Fluent UI](https://img.shields.io/badge/Fluent%20UI-React%20v9-0078d4?style=flat-square&logo=react)](https://react.fluentui.dev)
[![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

</div>

<!-- README-I18N:START -->

[English](./README.md) | **中文**

<!-- README-I18N:END -->

Litematica Preview 是一款适用于 Windows 和 Linux 桌面的 Minecraft 投影与结构查看器，支持在本地以 3D 方式预览 `.litematic`、`.schem`、`.schematic`、`.nbt`、`.snbt`、`.mcstructure`、`.mca` 和 `.nusn` 文件。

如果你使用 **macOS**，可以查看姊妹项目 **[LitematicaQL](https://github.com/Arcadi4/LitematicaQL)**，体验同样迅捷的预览与流畅的“快速查看”集成！

## 亮点

- 使用原生 wgpu（Windows 下使用 Direct3D 12，Linux 下使用 Vulkan）和 Nucleation，在本地以带纹理的 3D 视图预览八种 Minecraft 投影与结构格式。
- 可导入 Minecraft Java 资源包并保存在本地，按优先级排列已启用的资源包；缺失的资源依次从下方资源包及内置原版资源中获取。
- 通过紧凑解码和分块预览加载大型 `.litematic` 文件，避免常驻完整解压后的文件内容。
- 并行生成与上传几何数据，默认使用 32 方块分块，可调整分块大小和工作线程数，并显示渐进预览与可取消的加载进度。
- 在可复用的独立进程中解码，Windows 下限制其内存为 2 GiB。返回主页或取消加载会终止解码进程并释放缓存资源；大跨度、不分块的模型目前仍受 Core 0.1.4 紧凑坐标精度的限制。
- 可选择 Windows 安装包或便携版 ZIP，以及 Linux Debian 和 AppImage 安装包。

## 安装

从 [发布页面](https://github.com/Arcadi4/LitematicaPreview/releases) 下载最新版本：

### Windows x64

选择**安装包**（`LitematicaPreview-<version>-x86_64-setup.exe`），可按当前用户一键安装并从“关于”对话框更新；或选择**便携版 ZIP**（`LitematicaPreview-<version>-x86_64-portable.zip`），解压后运行 `LitematicaPreview.exe`，并保留解压后的文件；便携版也可就地更新。

需要 Windows 10 或 11 (x64)。若缺少 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)，安装程序会自动安装。首次启动后应用会询问一次是否注册文件类型以加入“打开方式”，不会更改默认应用。之后可在右上角菜单中修改，或在解压目录中使用 `.\current\LitematicaPreview.exe --register` / `--unregister`。

### Linux x64

通过 `sudo apt install ./LitematicaPreview-<version>-x86_64.deb` 安装 **Debian 包**（`LitematicaPreview-<version>-x86_64.deb`），或对 **AppImage**（`LitematicaPreview-<version>-x86_64.AppImage`）执行 `chmod +x` 后运行。

发布包以 Ubuntu 22.04 为目标，需要 GTK 3、WebKitGTK 4.1、Vulkan 加载器及兼容驱动，以及 `xdg-utils`；不保证较旧的发行版能运行发布版二进制文件。没有 Vulkan 设备时应用可以启动，但无法预览投影。Debian 包会声明这些依赖，安装桌面入口与 MIME 声明，不会修改默认应用；请在文件管理器中选择文件关联。

随 apt 仓库构建的 Debian 包还会安装已签名的源与密钥环（卸载软件包时会一并移除），后续版本可通过 apt 更新：

```bash
sudo apt update
sudo apt install litematica-preview
```

## 致谢

非常感谢 [@Nano112](https://github.com/Nano112) 的 [Nucleation](https://github.com/Schem-at/Nucleation) 项目为解析与网格生成管线提供支持。
