# SCIMxx

在 Apple 拼音候选窗口内重映射四个按键，候选数据、翻页和提交仍由 Apple 拼音处理。

| 输入 | 行为 |
| --- | --- |
| <kbd>&lt;</kbd> | 上一列 |
| <kbd>&gt;</kbd> | 下一列 |
| <kbd>[</kbd> | 首字 |
| <kbd>]</kbd> | 末字 |

候选窗口关闭时，四个按键保持原义。

## 构建

需要先在恢复模式中执行 `csrutil disable`。

```sh
cargo build --release
target/arm64e-apple-darwin/release/scimxx install
```

`scimxx` 与 `libscimxx_hook.dylib` 需要放在同一目录。可执行文件可以位于任意位置；安装命令会将 hook 复制到 `~/Library/Dictionaries/`，并请求管理员权限安装后台服务。

## 用法

- `scimxx install` 安装或更新
- `scimxx` 启动或重新启动
- `scimxx stop` 停止
- `scimxx uninstall` 卸载
- `scimxx version` 显示版本
