# Prompt Assets

本目录用于存放 **role prompts**（以 `@<role>` 选择的角色提示词）。

- 这些文件会在编译期被嵌入二进制（不依赖用户机器上的外部目录）。
- 运行时不会从磁盘读取这里的内容；修改后需要重新编译才会生效。

目录约定：

- `prompt/roles/<mode>.md`：与 mode 同名的角色提示词（例如 `prompt/roles/coder.md` 对应 `@coder`）。

