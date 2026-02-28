---
version: 1
name: install-git-self-bootstrap
mode: coder
subagent-fork: false
allowed_tools:
  - process/start
  - process/inspect
  - process/tail
  - process/follow
context:
  - argv: ["bash", "-lc", "echo shell=$SHELL; command -v omne || true; command -v git || true; git --version || true"]
    summary: "probe shell/omne/git availability"
    ok_exit_codes: [0]
---
目标：在当前机器自动补齐 `git`，并让本次对话后续命令立即可用；同时把路径持久化到 `~/.bashrc` 和 `~/.zshrc`。

执行约束（必须遵守）：

1. 仅允许通过 `omne toolchain bootstrap` 安装 git；禁止使用 `apt`/`yum`/`dnf`/`brew`/`pacman`/`choco`。
2. 只允许修改 `~/.bashrc`、`~/.zshrc`；禁止改其它 shell 配置文件。
3. 所有写入必须幂等（重复执行不能重复追加相同配置段）。

执行步骤（按顺序）：

1. 先检测 git 是否可用：
   - `command -v git`
   - `git --version`
2. 若 git 缺失，执行：
   - `omne toolchain bootstrap --json --strict`
   - 从 JSON 输出中提取 `managed_dir`。
3. 当前对话生命周期立即生效：
   - 在后续所有依赖 git 的命令前，先注入：
     - `export PATH="<managed_dir>:$PATH"; hash -r`
   - 立刻验证：
     - `command -v git`
     - `git --version`
4. 持久化到 `~/.bashrc` 和 `~/.zshrc`（两者都处理）：
   - 若文件不存在先创建空文件。
   - 只在不存在受管片段时追加以下内容（原样）：

```bash
# >>> omne managed toolchain >>>
export PATH="<managed_dir>:$PATH"
# <<< omne managed toolchain <<<
```

5. 重新应用 shell 配置：
   - 若当前 shell 是 bash 且存在 `~/.bashrc`，执行 `source ~/.bashrc`。
   - 若当前 shell 是 zsh 且存在 `~/.zshrc`，执行 `source ~/.zshrc`。
   - 无论当前 shell 类型如何，再用显式 PATH 注入执行一次 `git --version` 作为最终验证。

交付要求：

1. 输出最终状态：`git` 是否可用、`git --version`、`managed_dir`。
2. 输出修改结果：`~/.bashrc`、`~/.zshrc` 是否新增受管片段。
3. 若失败，给出最小可执行修复命令（不超过 3 条）。
