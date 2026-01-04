# git-touchfish-commit

一个 CLI 工具，用于在 `git commit` 后将 commit 时间修改为指定时间区间内的随机时间点，且保证晚于当前 HEAD 的 commit 时间。

## 使用方法

```bash
git tc set 09:00 17:00  # 设置时间区间，时间格式为 `HH:MM`，只需首次运行时设置
git tc -am "comment"    # 提交，commit 时间将自动设置为随机时间
git tc show             # 查看当前的时间区间
git tc amend            # 使用随机时间修改最后一次提交
git tc ...              # 任意 args，将原样传给 git commit
```
