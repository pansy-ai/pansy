## pansy 0.1.7

### 用手机操控家里的智能体

- 设置 › 网络与共享 › 手机和其他设备：扫码配对手机。在手机浏览器里开始、续接家里的 Codex 和 Claude Code 线程：审批、回答提问、插话、排队、停止，查看每个回合的改动，提交、推送、开 PR。
- 可以说话代替打字：输入框里的话筒把你说的话写到光标处（iOS、macOS，以及 HTTPS 下的浏览器）。
- 有线程在为手机运行时，电脑不会因为空闲而睡眠；合上盖子照样睡眠。
- 安装为后台服务后，窗口关掉，手机上的线程也照常运行。

### 智能体和供应商

- T3 Code 除了 Claude Code，也能经 pansy 的网关跑 Codex。
- 登录 Gemini Code Assist 前会说明风险：在 Gemini CLI 之外使用，Google 可能封号。
- Antigravity Cloak 已从精选列表中移除。

### 窗口

- 采用 T3 Code 的字号：整数像素、更轻的字重；macOS 上的文字更细。
- Arch Linux 和 Omarchy：`yay -S pansy-bin`。
