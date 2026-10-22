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

Litematica Preview 是一款适用于 Windows 和 Linux 桌面的 Minecraft 投影与结构查看器，改编自 [LitematicaQL](https://github.com/Arcadi4/LitematicaQL)。它支持在本地以 3D 方式预览 `.litematic`、`.schem`、`.schematic`、`.nbt`、`.snbt`、`.mcstructure`、`.mca` 和 `.nusn` 文件。

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

- **安装包 (`LitematicaPreview-<version>-win-x64-setup.exe`)**：按当前用户安装，无需管理员权限，并可注册所选的文件关联。
- **便携版 (`LitematicaPreview-<version>-win-x64-portable.zip`)**：解压压缩包并运行 `LitematicaPreview.exe`。请将 `Assets`、`Demos` 和 `Licenses` 目录保留在可执行文件同级目录下。

需要 Windows 10 或 11 (x64) 以及 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 和当前版本的 Windows 10 已预装；若缺失，安装程序会自动下载）。

> [!NOTE]
> 若需在安装后更改文件关联，请打开右上角菜单并选择 **Set as default app…** 或 **Remove file associations**。便携版也可以在 PowerShell 中通过 `.\LitematicaPreview.exe --register` 和 `.\LitematicaPreview.exe --unregister` 注册或取消注册。

### Linux x64

- **Debian 安装包 (`LitematicaPreview-<version>-linux-x64.deb`)**：通过 `sudo apt install ./LitematicaPreview-<version>-linux-x64.deb` 安装。
- **AppImage (`LitematicaPreview-<version>-linux-x64.AppImage`)**：通过 `chmod +x` 添加执行权限后运行。

Linux 发布包以 Ubuntu 24.04 为目标，需要 GTK 3、WebKitGTK 4.1、Vulkan 加载器及兼容驱动，以及 `xdg-utils`。Debian 安装包会声明这些依赖。不保证较旧的发行版能够运行由发布工作流构建的二进制文件。

Debian 安装包会安装桌面入口与 MIME 声明，但不会修改默认应用。请通过桌面文件管理器选择文件关联；Linux 下不显示 Windows 的文件关联注册命令。

## 资源包

打开 **Settings → Texture packs** 导入 Minecraft Java 版资源包 ZIP 文件。用开关启用资源包，再用箭头按钮调整顺序，最上方的优先级最高。新导入的资源包位于自定义资源包的最低优先级。

缺失的纹理、模型和方块状态依次从下方资源包及内置原版资源中获取。修改后，当前预览会重新加载。已保存的资源包和原有的单资源包选择会保留，移除已保存的副本不会删除原始 ZIP 文件。

## 支持的格式

| 扩展名         | 文件格式                                |
| -------------- | --------------------------------------- |
| `.litematic`   | Litematica                              |
| `.schem`       | Sponge schematic（海绵投影）            |
| `.schematic`   | MCEdit                                  |
| `.nbt`         | Java 版结构方块（Structure block）      |
| `.snbt`        | 结构 SNBT（支持花括号或方括号方块状态） |
| `.mcstructure` | 基岩版结构（Bedrock structure）         |
| `.mca`         | Minecraft Anvil 区域文件                |
| `.nusn`        | Nucleation 快照（Nucleation snapshot）  |

## 开发

解码、网格打包和纹理首次使用时的准备工作由
[LitematicaPreview Core](https://github.com/Arcadi4/litematica-preview-core) 完成。
宿主将准备好的字节直接传输进原生 wgpu 渲染器，不会转换顶点属性或重排索引。Windows 通过位于 WebView2 下方的 Direct3D 12 交换链直接呈现。Linux 则使用 Vulkan 离屏渲染，并将帧像素交给透明 WebKitGTK 下方的 GTK DrawingArea 绘制。X11/Wayland 的合成交由 GTK 处理，代价是每次实际渲染一帧都需要一次 GPU 回读；几何数据不会跨越 WebView 桥接层。

GPU 上传同时受批次数量和几何、纹理字节合计 64 MiB 的预算约束。超过预算的单个批次可独占上传窗口；该预算并非模型总大小或显存上限。返回主页会释放场景几何及深度、MSAA 附件，后两者仅在有可见几何需要时分配。较小分块能降低网格生成过程的内存峰值，增加工作线程则以更多内存换取吞吐量。

Linux 呈现使用有界的最新帧邮箱与可复用像素缓冲区。GPU 回读等待在专用线程中进行，GTK 对象只在主线程访问。窗口尺寸变化时会丢弃尺寸不匹配的旧帧，而不是拉伸显示；空闲窗口不会启动渲染定时器。

在开发模式下运行桌面应用：

```bash
pnpm install --frozen-lockfile
pnpm run tauri:dev
```

运行前端和 Rust 检查（与 CI 一致）：

```bash
pnpm run check
```

使用尚未发布的本地核心库时，先更新锁文件中的依赖来源：

```bash
LP_CORE_PATH=../litematica-preview-core pnpm run resolve:host
LP_CORE_PATH=../litematica-preview-core pnpm run test:host
```

提交前，不设置该环境变量运行 `pnpm run resolve:host`，即可恢复使用已发布的依赖。

### 构建 Windows 版本

- Node.js 24 LTS 与 pnpm
- Rust 稳定版工具链 (`x86_64-pc-windows-msvc`)
- Visual Studio C++ Build Tools（使用 C++ 的桌面开发、x64 MSVC、Windows SDK）

```bash
git clone https://github.com/Arcadi4/LitematicaPreview.git
cd LitematicaPreview

rustup target add x86_64-pc-windows-msvc
pnpm run build:win
```

构建产物（`*-setup.exe`、`*-portable.zip` 以及可直接运行的 `win-x64/` 目录）会输出到 `artifacts/`。
Cargo 默认使用工作区的 `target/` 目录；可通过 `CARGO_TARGET_DIR` 指定其他目录。发布工作流在测试与打包时共用此目录及 Windows MSVC 目标平台。
Cargo 包通过 `default-run` 指定 `LitematicaPreview`，确保同时存在 `pressure` 可执行程序时 Tauri 仍能选中主程序。压力工具需通过 `pnpm run pressure:host -- ...` 显式运行。

### 构建 Linux 版本

使用安装了 Node.js 24、pnpm 12 与 Rust 稳定版的 Linux x64 主机。在 Ubuntu 24.04 上安装原生构建依赖：

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev libssl-dev librsvg2-dev \
  libayatana-appindicator3-dev libvulkan-dev mesa-vulkan-drivers patchelf
```

```bash
pnpm install --frozen-lockfile
rustup target add x86_64-unknown-linux-gnu
pnpm run build:linux
```

Tauri 构建会准备前端并打包资源。带版本号的 `.deb` 与 `.AppImage` 文件输出到 `artifacts/`；可通过 `CARGO_TARGET_DIR` 指定 Cargo 输出目录。Linux 包的校验和通过 `pnpm run checksums --linux` 生成，同时传入 `--win --linux` 则生成合并发布包的校验和。

将 Linux 发布版视为稳定版本之前，需在 X11 与 Wayland 桌面分别验证实时合成：首次加载、返回主页或取消、调整大小与最大化或全屏、HiDPI、指针与键盘输入、原生对话框模态关系、最小化与恢复，以及长时间渲染时的半透明叠加层。容器或离屏检查不能证明真实桌面和驱动的兼容性。

## 致谢

非常感谢 [@Nano112](https://github.com/Nano112) 的 [Nucleation](https://github.com/Schem-at/Nucleation) 项目为解析与网格生成管线提供支持，以及感谢 [LitematicaQL](https://github.com/Arcadi4/LitematicaQL) 提供最初的 macOS 实现。
