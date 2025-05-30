# git-touchfish-commit

一个 CLI 工具，用于在 `git commit` 后将 commit 时间修改为指定时间区间内的随机时间点。

## 安装

工具主要面向程序员群体，因此不再赘述安装方式。

## 使用方法

```bash
git tc set 09:00 17:00  # 设置时间区间，时间格式为 `HH:MM`，只需首次运行时设置
git tc -a -m "comment"  # 提交，commit 时间将自动设置为随机时间
```
