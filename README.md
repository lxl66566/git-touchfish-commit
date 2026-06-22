# git-touchfish-commit

一个 CLI 工具，用于在 `git commit` 后将 commit 时间修改为指定时间区间内的随机时间点，且保证晚于当前 HEAD 的 commit 时间。

## 使用方法

```bash
# 配置时间区间 (HH:MM)，默认写入 --global，所有仓库共享
git tc set 00:00 04:00

# 仅对当前仓库设置
git tc set --local 00:00 04:00

# 查看当前生效的时间区间
git tc show

# 提交 (任意 git commit 参数透传)，commit 时间将自动设置为随机时间
git tc -am "comment"

# 使用随机时间修改最后一次提交。注意，是 amend 不是 --amend
git tc amend
```

## 特性

- 同时改写 author date 与 committer date
- 保证单调递增：每次生成的时间严格晚于 HEAD 的时间
- 配置走 git config：复用 git 的 system / global / local 三级配置
