<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview 图标" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>适用于 Windows、macOS 和 Linux 的 Minecraft 投影与结构查看器</strong></p>

[![Rune](https://img.shields.io/badge/UI-Rune-5546eb?style=flat-square)](https://github.com/anomalyco/rune)
[![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

</div>

<!-- README-I18N:START -->

[English](./README.md) | **中文**

<!-- README-I18N:END -->

Litematica Preview 是一款适用于 Windows、macOS 和 Linux 的原生桌面应用，支持在本地以 3D 方式预览 `.litematic`、`.schem`、`.schematic`、`.nbt`、`.snbt`、`.mcstructure`、`.mca` 和 `.nusn` 文件。

## 亮点

- 在同一个窗口中用标签页打开多个投影。打开文件夹或单个文件时，可以通过已缓存的缩略图浏览同目录中的投影。
- 支持旋转、平移、缩放和旁观飞行。可在偏好设置中为两种模式设置快捷键，并调整旁观模式的视野和鼠标灵敏度。
- 可将预览复制到剪贴板，或保存为 PNG、JPEG 图片。支持按优先级叠加 Minecraft Java 资源包，缺失纹理使用内置资源。
- 大型投影可以边加载边预览，支持调整分块大小和工作线程数。

## 安装

从 [发布页面](https://github.com/Arcadi4/LitematicaPreview/releases) 下载最新版本：

### Windows x64

选择**安装包**（`LitematicaPreview-<version>-x86_64-setup.exe`）可自选安装目录，也可以注册支持的文件类型。使用**便携版 ZIP**（`LitematicaPreview-<version>-x86_64-portable.zip`）时，解压后运行 `LitematicaPreview.exe`。

### macOS

打开适用于 Apple 芯片（`aarch64`）或 Intel（`x86_64`）的 `.dmg`，将 Litematica Preview 拖入“应用程序”。也可运行 `brew install --cask Arcadi4/tap/litematica-preview`。如需在访达中使用“快速查看”，可安装 [LitematicaQL](https://github.com/Arcadi4/LitematicaQL)。

### Linux x64

通过 `sudo apt install ./LitematicaPreview-<version>-x86_64.deb` 安装 **Debian 包**（`LitematicaPreview-<version>-x86_64.deb`），或对 **AppImage**（`LitematicaPreview-<version>-x86_64.AppImage`）执行 `chmod +x` 后运行。

Linux 需要兼容的 Vulkan 驱动。AppImage 通过内置的 [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate/issues) 更新。Debian 包不会在应用内更新；稳定版会安装签名的 apt 源，后续可用以下命令更新：

```bash
sudo apt update
sudo apt install litematica-preview
```

### 更新来源

Windows 和 macOS（包括通过 Homebrew 安装的版本）可在偏好设置中检查更新。中国大陆用户建议选择 **Cloudflare**，其他地区可使用 **GitHub**。关闭应用内更新后，更新来源选项将隐藏。

如需从源码构建，请参阅[构建与发布](docs/build.md)。

## 致谢

非常感谢 [@Nano112](https://github.com/Nano112) 的 [Nucleation](https://github.com/Schem-at/Nucleation) 项目为解析与网格生成管线提供支持。
