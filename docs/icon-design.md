# 启动图标

Android Bug Desk 使用明亮蓝绿配色：蓝色圆角底板、绿色缺陷图形和白色勾选。以粗轮廓和少量元素保证小尺寸识别。

## 文件与再生成

源图：`scripts/app-icon.png`，1254×1254 RGBA PNG，透明外边距。由内置 image_gen 生成，没有使用 CLI/API fallback。
桌面产物：`src-tauri/icons/`。配置：`src-tauri/tauri.conf.json`。

在已安装项目依赖的目录运行：

```sh
npm exec -- tauri icon scripts/app-icon.png --output src-tauri/icons
```

Tauri 也会生成被现有 .gitignore 排除的 Android/iOS 图标；本次交付仅含桌面图标。

参考项目 OkHttpDebugDesktop 的 bundle.icon 列表、PNG 尺寸、ICO 帧尺寸和 ICNS 类型。没有复制参考项目的品牌图形。

- PNG：32、64、128、256（128@2x）、512。
- Windows ICO：16、24、32、48、64、256。
- macOS ICNS：包含 16–1024 的常用表示及 Retina 表示。
- Windows Appx PNG：StoreLogo 50；Square 30、44、71、89、107、142、150、284、310。

## 验证与范围

PNG 实际尺寸/RGBA、ICO 帧集合、ICNS 类型集合与参考项目一致（容器顺序不要求相同）。系统 sips 成功读取 ICO/ICNS，透明通道存在；全部配置引用均存在，32×32 和 512×512 已目视检查；git diff --check 通过。

本次只更新图标资源及配置，不改业务逻辑，不触发 GitHub Actions，不生成或安装新的应用包。三平台实际安装显示、Dock/任务栏图标缓存效果尚未验证；已有安装包不会因源码变化自动更新。

## 最终生成提示词

```text
Use case: logo-brand
Asset type: production launcher icon for Android Bug Desk, a desktop Android bug collection and QA application on macOS, Windows and Linux.
Primary request: create one beautiful bright minimal app icon, immediately recognizable at small sizes.
Subject: a single friendly lime-green bug emblem with a simple white checkmark integrated in its body, communicating bug testing and verification. Compact rounded geometric silhouette, two short antennae, just a few broad symmetrical legs, no face, no detailed illustration.
Scene/backdrop: a luminous sky-blue rounded square tile, with a restrained blue-to-cyan gradient, clean and fresh. True transparent canvas outside the rounded tile, no white square behind it.
Composition/framing: one centered icon, front-on and orthographic, square 1024x1024 canvas. Tile occupies about 82 percent of canvas width with equal generous transparent margins. Emblem has generous inner padding, a bold clear silhouette and high contrast.
Style/medium: polished minimal graphic app identity, flat vector-like shapes rendered as raster, crisp smooth edges, subtle depth confined within tile, bright simple friendly visual mood.
Constraints: one icon only, no lettering, no text, no watermark, no mockup, no device frame, no surrounding objects, no thin strokes, no dark palette, no chrome, no complex 3D rendering. Outside tile must be genuinely transparent.
```

