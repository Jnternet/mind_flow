#!/usr/bin/env bash
# 把当前工作区发布到 GitHub。
#
#   GITHUB_TOKEN=xxx bash scripts/publish.sh "feat: 说明这次改了什么"
#   bash scripts/publish.sh --dry-run "chore: 只演练不推送"
#
# 为什么不用工作区里的 .git：这个开发容器把 <项目>/.git 挂成了只读 tmpfs，
# 所以脚本会把工作区同步到一个可写的镜像仓库（默认 /tmp/mind_flow_publish），
# 在那里提交并推送。真正的代码始终以工作区为准，镜像只是中转。
#
# 凭据优先级：$GITHUB_TOKEN（推荐，放环境变量里）> ~/.git-credentials。
# 需要的权限：细粒度 token 勾「Contents: Read and write」；经典 token 勾 repo。
# 脚本不会把 token 写进仓库，也不会写进 .git/config。
set -euo pipefail
cd "$(dirname "$0")/.."
WORKSPACE="$PWD"

REMOTE="${MIND_FLOW_REMOTE:-https://github.com/Jnternet/mind_flow.git}"
MIRROR="${MIND_FLOW_MIRROR:-/tmp/mind_flow_publish}"
BRANCH="${MIND_FLOW_BRANCH:-main}"
DRY_RUN=""
MESSAGE=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN="--dry-run" ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) MESSAGE="$arg" ;;
  esac
done
if [[ -z "$MESSAGE" ]]; then
  echo "用法：GITHUB_TOKEN=xxx bash scripts/publish.sh \"chore: 提交说明\"" >&2
  exit 2
fi

# ---------------------------------------------------------------- 准备镜像仓库
if [[ ! -d "$MIRROR/.git" ]]; then
  echo "-- 初始化镜像仓库 $MIRROR"
  git clone -q "$REMOTE" "$MIRROR"
  git -C "$MIRROR" config user.name "${GIT_AUTHOR_NAME:-Jnternet}"
  git -C "$MIRROR" config user.email "${GIT_AUTHOR_EMAIL:-jnternet@users.noreply.github.com}"
fi

# ---------------------------------------------------------------- 同步工作区
echo "-- 同步工作区 → 镜像"
tar -C "$WORKSPACE" -cf - \
  --exclude=./target --exclude=./vendor --exclude=./dist --exclude=./data \
  --exclude=./.git --exclude=./.agents --exclude=./.codex \
  . | tar -C "$MIRROR" -xf -
# 工作区里已经删掉的文件，在镜像里也要删掉（tar 只覆盖不删除）
while IFS= read -r -d '' path; do
  if [[ ! -e "$WORKSPACE/$path" ]]; then
    git -C "$MIRROR" rm -q -f --ignore-unmatch -- "$path"
  fi
done < <(git -C "$MIRROR" ls-files -z)

if git -C "$MIRROR" diff --quiet && git -C "$MIRROR" diff --cached --quiet \
   && [[ -z "$(git -C "$MIRROR" ls-files --others --exclude-standard)" ]]; then
  echo "-- 没有新改动，直接检查远端"
else
  git -C "$MIRROR" add -A
  git -C "$MIRROR" commit -q -m "$MESSAGE"
  echo "-- 已提交：$(git -C "$MIRROR" log -1 --oneline)"
fi

# ---------------------------------------------------------------- 推送
push_args=(-C "$MIRROR")
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  # 用临时的 credential helper 从环境变量取 token：token 不会出现在命令行或 .git/config 里
  push_args+=(-c "credential.helper=!f(){ echo username=x-access-token; echo password=\$GITHUB_TOKEN; }; f")
else
  push_args+=(-c credential.helper=store)
fi

echo "-- 推送到 $REMOTE ($BRANCH)${DRY_RUN:+（演练）}"
set +e
git "${push_args[@]}" push $DRY_RUN origin "HEAD:$BRANCH" 2>&1 | sed 's/^/   /'
status=${PIPESTATUS[0]}
set -e
if [[ $status -ne 0 ]]; then
  cat >&2 <<'EOF'

推送失败。常见原因：
  - token 无效（401 Bad credentials）：换一个没抄错的 token。
  - 403 / "Resource not accessible by personal access token"：
      * 细粒度 token：在 token 设置里把本仓库加入，并勾上「Contents: Read and write」；
      * 经典 token：勾上 repo。
  - 网络：本机 github.com 需要走代理，脚本沿用 git 的 http.proxy 配置。
EOF
  exit $status
fi

echo "-- 完成：$(git -C "$MIRROR" log -1 --format='%h %s')"
