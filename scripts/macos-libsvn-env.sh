#!/bin/sh
#
# 为 macOS 的 libsvn 后端生成 svn_<lib>-1.pc pkg-config 文件，
# 并导出 cargo build 需要的 PKG_CONFIG_PATH / LIBRARY_PATH。
#
# Homebrew 的 subversion 是 keg-only 且不附带 .pc 文件，而
# subversion crate 的 subversion-sys 通过 pkg-config 探测 libsvn_*。
#
# 用法（在任意目录执行均可，脚本会定位到本仓库根目录）：
#   source scripts/macos-libsvn-env.sh
#
# 依赖：brew install subversion apr apr-util utf8proc gettext lz4 zlib
#       （subversion/apr/apr-util 需在构建前安装）
#
set -e

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

SVN_PREFIX=$(brew --prefix subversion)
APR_PREFIX=$(brew --prefix apr)
APR_UTIL_PREFIX=$(brew --prefix apr-util)

PC_DIR="$REPO_ROOT/target/svn-pc"
mkdir -p "$PC_DIR"

VERSION=$("$SVN_PREFIX/bin/svn" --version --quiet 2>/dev/null || true)
if [ -z "$VERSION" ]; then
  VERSION=$(perl -ne 'if(/SVN_VER_(MAJOR|MINOR|PATCH)\s+(\d+)/){$v{$1}=$2} END{print "$v{MAJOR}.$v{MINOR}.$v{PATCH}"}' \
    "$SVN_PREFIX/include/subversion-1/svn_version.h" 2>/dev/null || true)
fi
[ -n "$VERSION" ] || VERSION="1.14.5"

for lib in client subr delta diff repos fs wc ra; do
  cat > "$PC_DIR/svn_${lib}-1.pc" <<EOF
prefix=$SVN_PREFIX
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include/subversion-1

Name: libsvn_${lib}
Description: Subversion library (${lib})
Version: ${VERSION}
Requires: apr-1, apr-util-1
Libs: -L\${libdir} -lsvn_${lib}-1
Cflags: -I\${includedir}
EOF
done

export PKG_CONFIG_PATH="$PC_DIR:$APR_PREFIX/lib/pkgconfig:$APR_UTIL_PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LIBRARY_PATH="$(brew --prefix utf8proc)/lib:$(brew --prefix gettext)/lib:$(brew --prefix lz4)/lib:$(brew --prefix zlib)/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"