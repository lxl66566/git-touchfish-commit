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

# 批量改写区间内所有提交：在整个区间的时间跨度与时间窗口的交集内一次性随机取点，
# 保证严格递增；区间后的提交会被原样重建 (仅父指针变化，时间不变)
git tc amend HEAD~5..HEAD
git tc amend abc123..HEAD
```

## 特性

- 同时改写 author date 与 committer date
- 保证单调递增：每次生成的时间严格晚于 HEAD 的时间；批量改写时整体落在
  边界父提交与区间后首个提交之间，与前后历史保持单调
- 配置走 git config：复用 git 的 system / global / local 三级配置
- 批量改写会丢弃被改写提交的 GPG 签名 (时间变化后签名必然失效)
