# SCIM++

macOS 26 中文输入法增强。

### 功能

1. 重映射快捷键<br/>
    <kbd>&lt;</kbd> = <kbd>原 上一列 [</kbd><br/>
    <kbd>&gt;</kbd> = <kbd>原 下一列 ]</kbd><br/>
    <kbd>[</kbd> = <kbd>原 候选词首字 ⇧ + ⌘ + [</kbd><br/>
    <kbd>]</kbd> = <kbd>原 候选词末字 ⇧ + ⌘ + ]</kbd><br/>

<sub>
*<sup>1.</sup> 需要关闭 SIP 才能注入。安装命令会将 hook 复制到 ~/Library/Dictionaries 并安装后台服务。<br/>
</sub>

### 用法

```zsh
brew tap zetaloop/zetaloop; brew trust zetaloop/zetaloop
brew install scimxx
```

- `scimxx install` 安装服务
- `scimxx start` 启动 / 重启
- `scimxx stop` 停止
- `scimxx uninstall` 卸载
- `scimxx version` 显示版本

_顺便一提：[ChsIME++](https://github.com/zetaloop/ChsIMExx)_
