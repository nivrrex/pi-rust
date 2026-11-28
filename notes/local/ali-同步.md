# ali 云盘 git 同步

- 远端：`ssh://root@8.153.158.12:8022/srv/git/pi-rust.git`（remote 名 `ali`）
- 裸库位置：`/srv/git/pi-rust.git`；`HEAD` 已指向 `refs/heads/pi-rust-main`
- 当前分支 `pi-rust-main` 的上游就是 `ali/pi-rust-main`

## 日常

```sh
git add -A && git commit -m "..."   # 本地提交
git push                            # 推到 ali
```

拉取/对齐：`git fetch ali && git log --oneline HEAD..ali/pi-rust-main`

## 全量同步（分支 + tag）

```sh
git push --all ali && git push --tags ali
```

## 注意

- 4 个分支：`pi-rust-main`（主线，最新）、`f63-publish`、`publish-v0.87.1`、`recover`
- 2 个 tag：`v0.87.1`、`first-upload-v0.87.1`
- 没有接 GitHub 的 remote——GitHub 发布由 `tools/publish/publish.sh` 单独推，
  与本同步互不影响
