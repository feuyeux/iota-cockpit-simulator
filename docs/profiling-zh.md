# Desktop CPU 与内存火焰图采集

脚本附加到**已经运行**的 Cockpit Desktop、`cockpit-simulator` 或其他子进程。由于 Tauri Host、Simulator 和 WebView 是不同进程，建议分别采集，避免把热点归到错误的运行时。

## 1. Linux 与 macOS

```bash
tools/profile-desktop.sh
tools/profile-desktop.sh cpu
tools/profile-desktop.sh memory
```

---

## 2. Windows（PowerShell）

```powershell
.\tools\profile-desktop.ps1
.\tools\profile-desktop.ps1 -Type cpu
.\tools\profile-desktop.ps1 -Type memory
```

---

## 3. 默认行为

- 自动安装缺失工具，并尝试升级到包管理器提供的最新版本；Linux 需要 `sudo`，Windows 的 WPR 通常需要管理员终端。
- macOS 的系统采样工具随 Xcode/macOS 更新；若系统弹出 Command Line Tools 安装窗口，完成安装后重新运行脚本。
- macOS 内存采集会自动切到 `tauri build --debug` 生成的可附加签名包，再用 `xctrace` 采集；这是绕开 `tauri dev` 里未附加 entitlement 的稳定路径。首次运行会先构建 bundle。脚本仍会在每次 CPU/内存采集开始前确认目标 PID 仍在运行。
- 不传采集类型时，脚本顺序生成 CPU 和内存两份报告；传 `cpu` 或 `memory` 时只生成对应报告。
- 无需定位 `PID`。脚本优先选择 `Cockpit Simulation` 或 `cockpit-desktop` 主进程；找不到主进程时自动回退到 `cockpit-simulator`。
- 如需诊断指定子进程，仍可使用 `--pid`、`--process`、`-ProcessId` 或 `-ProcessPattern` 覆盖自动选择结果。
- 结果写入仓库根目录的 `profile-results/`。该目录可能包含运行数据，不应提交。
- 离线或受管环境可传 `--no-update`（PowerShell 为 `-NoUpdate`），但依赖必须已经存在。

---

## 4. 输出格式

| 平台 | CPU | 内存分配调用栈 |
| --- | --- | --- |
| Linux | `perf.data`、折叠栈和 SVG | Heaptrack 文件；用 `heaptrack_gui` 的 Flame Graph 打开 |
| macOS | `sample` 原始栈、折叠栈和 SVG | Instruments Allocations `.trace`；在 Call Tree/Flame Graph 查看 |
| Windows | WPR `.etl`；在 WPA 的 Flame Graph 查看 | WPR Heap `.etl`；在 WPA 的 Heap Allocations/Flame Graph 查看 |

---

## 5. 能力边界

**内存火焰图**表示分配调用栈，不是 `RSS` 随时间变化的折线。`React/WebView` 的 `JavaScript` 堆还应使用 `WebView DevTools` 的 `Memory Heap Snapshot`。原生采集无法完整显示 `React` 对象保留关系。
