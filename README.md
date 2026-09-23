<div align="center">

# cc-switch-ppbind

### 给 CC Switch 增加“项目级上游渠道绑定 + 强制路由模型”

一个面向 Codex / Claude Code 多项目开发的 CC Switch 定制分支。

</div>

<p align="center">
  <a href="https://github.com/xwyjh3612/cc-switch-ppbind/releases/latest/download/PPBind_3.20.4_x64-setup.exe"><strong>下载最新版 Windows x64 安装包</strong></a>
  ·
  <a href="https://github.com/xwyjh3612/cc-switch-ppbind/releases">查看全部版本</a>
</p>

> `ppbind` = **Project Provider Bind**。  
> 本仓库基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 开发，保留原有供应商管理、代理接管、会话管理、MCP、Prompts、Skills 等能力，并重点增加按项目文件夹绑定上游渠道和强制指定最终模型的能力。  
> 原版 README 已归档到 [`docs/original-readme`](docs/original-readme/)。

## 为什么需要这个分支

在同一个 Codex 或 Claude Code 客户端里，不同项目经常需要走不同上游：

- A 项目使用高性能渠道和顶级模型，适合复杂项目开发；
- B 项目使用高性价比渠道和常规模型，适合简单审核任务；
- C 项目忽略客户端当前模型，始终将请求重定向到指定模型；
- 切换项目时自动使用项目对应的渠道和模型配置，无需反复修改全局供应商；
- 可以按项目复杂度灵活配置：复杂项目使用高规格模型，简单任务使用常规模型。

传统做法是每次进入项目后手动切换全局供应商，但 Codex 的模型切换和供应商切换并不总是可靠。`cc-switch-ppbind` 把绑定关系放到项目维度：**全局供应商仍然保留，只有命中的项目请求使用项目指定渠道。**

## 核心功能

### 1. 项目管理

- 在“会话管理”旁增加“项目管理”入口；
- 按会话使用的项目文件夹聚合项目，而不是按单个会话展示；
- 支持 Codex 和 Claude Code 项目类型切换；
- 项目卡片展示项目目录、最近使用的模型和当前项目供应商；
- 项目默认保持收起，展开后显示最近 5 条会话；会话较多时可切换为显示全部；
- 每个会话可以直接指定供应商和模型，默认跟随项目管理配置。

### 2. Provider 独立上游代理

- 每个 Provider 可以单独配置上游 HTTP/HTTPS/SOCKS5 代理；
- 留空时跟随全局代理配置；
- 代理配置只影响该 Provider 的上游请求，不改变其他 Provider；
- Provider 级代理客户端会复用连接池，避免每次请求重新建立连接。

### 3. 项目级供应商绑定

- 每个项目可以单独选择上游供应商；
- 第一项是“默认供应商”，表示继续使用 PPBind 全局路由；
- 项目绑定只影响该项目，不修改全局当前供应商；
- 未绑定或无法识别项目时，继续走原有全局供应商和故障转移逻辑；
- 供应商被删除或项目路径异常时，界面会显示异常状态，不会静默切换到错误渠道。

路由优先级：

```text
会话级指定供应商 > 项目级指定供应商 > 全局当前供应商 > 原有故障转移/默认逻辑
```

### 4. 会话级供应商与模型覆盖

- 展开项目后，每条会话可以单独选择供应商和强制模型；
- 未指定时显示“跟随项目”，分别继承项目的供应商或模型设置；
- 会话优先于项目；会话没有覆盖的部分继续从项目配置补齐，项目也没有时再走全局路由；
- 默认只显示最近 5 条会话，超过后可展开显示全部；
- 修改或清除项目供应商、项目强制模型以及“全部恢复默认”时，会二次确认并清除该项目、该客户端下的会话级覆盖。

### 5. Codex 会话识别

Codex 请求会从请求头中的稳定会话 ID 识别会话，再通过本地会话元数据把会话映射到项目 `cwd`：

```text
session_id / x-session-id
        ↓
本地会话索引
        ↓
项目 cwd / projectDir
        ↓
项目供应商绑定
```

会话 ID 到项目目录的查询支持缓存，避免每个请求都重复扫描 SQLite。已有会话级路由时可直接按会话 ID 命中；没有会话级记录且项目映射尚未建立时，会先按全局路由处理。

### 6. 强制路由模型

每个项目可以在指定供应商之后，再开启“强制路由模型”。

- 强制模型是最终出站模型的权威覆盖；
- 优先级高于 Codex 当前模型、供应商模型映射和兼容转换；
- 项目可以选择“关闭强制路由模型”，恢复普通路由；
- 新项目默认关闭；
- 没有选择项目供应商时，不能开启强制模型；
- Codex 和 Claude Code 各自维护一套模型候选列表，同一客户端内的项目共用；
- 候选列表支持搜索过滤、新增和删除；
- 只有输入内容没有匹配项时，才显示“新增此模型”操作；
- 删除候选不会删除项目已经保存的模型值，避免运行中的项目被静默切换。

模型优先级：

```text
会话强制模型 > 项目强制模型 > 客户端请求模型 > 供应商模型映射/默认模型
```

### 7. 最近模型

项目卡片会显示该项目最近一次实际使用的模型，便于快速确认路由是否符合预期。

### 8. 下拉框体验

- 供应商和模型选择统一为紧凑型可搜索下拉框；
- 无内边距，选项直接铺满；
- 当前选中项保持原位并高亮；
- 列表最大高度会根据当前可用窗口空间动态缩短；
- 窗口较小时，下拉框不会超出顶部页头或窗口底部；
- 空间不足时列表内部滚动，不会出现上方选项看不到、无法点击的问题。

### 9. 独立更新

应用内“检查更新”已切换为 PPBind 自己的 GitHub Releases 更新链路，不再检查官方 CC Switch 更新源：

- 启动后自动检查，也可在“关于”页手动检查；
- 有新版本时展示 PPBind Release Notes 和下载入口；
- Windows 下载 NSIS 安装包，校验 GitHub 提供的 SHA-256 后静默安装并自动重启；
- 安装过程只结束 `ppbind.exe`，不会结束官方 `cc-switch.exe`；
- 便携版点击更新会打开 PPBind Releases 页面。

发布仓库需要保持公开。更新检查直接读取 GitHub Release 页面和 `SHA256SUMS.txt`，不调用有匿名频率限制的 GitHub API，也不会在安装包内保存任何 Token。

## 使用流程

1. 在 PPBind 中配置好全局供应商（首次启动可按提示导入官方 CC Switch 数据）。
2. 为 Codex 或 Claude Code 开启 PPBind 代理接管。
3. 打开“项目管理”。
4. 选择项目类型，再从项目卡片中选择供应商；如需清空当前类型的全部项目绑定，可点击“全部恢复默认”并二次确认。
5. 如需固定最终模型，点击模型下拉框并选择模型。
6. 如需只调整某个会话，展开项目卡片，在该会话的供应商或模型下拉框中选择“跟随项目”以外的配置；修改项目级配置时按提示确认重置会话覆盖。
6. 选择模型后自动开启强制路由；选择第一项即可关闭。
7. 没有配置项目绑定的请求继续使用全局默认供应商。

## 路由行为

- 项目供应商绑定是“指定渠道”语义，不修改全局供应商配置。
- 项目未命中时不会影响其他项目。
- 项目强制模型只在该项目已经存在供应商绑定时生效。
- 强制模型会覆盖最终出站 JSON 中的 `model`。
- 项目路径会标准化后匹配，避免大小写、分隔符或尾斜杠造成重复项目。
- 同一路径下的 Codex 和 Claude Code 配置分别保存，避免不同客户端的供应商 ID 冲突。

## 安装

### 双击安装

- [直接下载最新版 Windows x64 安装包](https://github.com/xwyjh3612/cc-switch-ppbind/releases/latest/download/PPBind_3.20.4_x64-setup.exe)
- [查看 GitHub Releases 中的全部版本](https://github.com/xwyjh3612/cc-switch-ppbind/releases)

构建后的 Windows 安装包位于：

```text
release/PPBind_3.20.4_x64-setup.exe
```

双击安装包即可安装，默认安装到 %LOCALAPPDATA%\PPBind。

PPBind 与官方 CC Switch 使用不同进程名、安装目录、数据目录和 ppbind:// 协议，可以同时安装，但不能同时运行。每次启动 PPBind 都会检查官方 CC Switch；如果官方程序仍在运行，PPBind 会阻止启动并提示先完全退出，且不会自动结束官方程序，以免中断用户正在进行的会话。退出官方程序后打开 PPBind，首次启动会询问是否只读复制官方 CC Switch 数据，复制过程不会修改原数据库。

### 本地构建并安装

```powershell
pnpm install
pnpm build:release
# 本地快速打包：只生成 NSIS 安装包，体积稍大，但后续增量构建快很多
pnpm build:release-fast
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-local.ps1
```

`build:release` 会把 NSIS/MSI 安装包和便携版复制到项目的 `release/` 目录。`build:release-fast` 使用独立的 `target-fast/` 缓存、关闭 LTO，并只构建 NSIS 安装包；产物同样复制到 `release/`，适合本地频繁验证。

## 开发

```powershell
pnpm install
pnpm dev
pnpm debug:run  # debug 增量构建；停止旧进程、覆盖安装、启动一条命令完成
pnpm typecheck
pnpm test:unit
pnpm format:check
```

`pnpm debug:run` 只用于开发调试，不生成压缩安装包。功能验收通过后再执行 `pnpm build:release` 生成正式安装包。

## 文档

- [项目管理设计与实现](docs/project-manager.md)
- [原版英文 README](docs/original-readme/README.md)
- [原版中文 README](docs/original-readme/README_ZH.md)
- [原版日文 README](docs/original-readme/README_JA.md)
- [原版德文 README](docs/original-readme/README_DE.md)

## 上游与许可证

本项目基于上游 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 修改，遵循原项目 MIT License。原项目版权和贡献归原作者及贡献者所有；本分支新增代码同样按 MIT License 发布。
