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
sudo cp target/arm64e-apple-darwin/release/scimxx /usr/local/bin/
cp target/arm64e-apple-darwin/release/libscimxx_hook.dylib ~/Library/Dictionaries/
```

## 运行

`scimxx` 等待 `SCIM_Extension`，取得管理员授权后加载 hook，并在输入法进程重启时重新加载。`build.loop.scimxx.plist` 可用于登录时启动 daemon。

hook 日志写入 `~/Library/Dictionaries/scimxx-hook.log`。
