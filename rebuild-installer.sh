#!/bin/bash
# 重新打包安装程序脚本
# 使用 E: 盘作为缓存目录（C: 盘空间不足）

set -e

echo "======================================"
echo "Novel Generate Team 重新打包流程"
echo "======================================"
echo ""

# 设置环境变量（使用 E: 盘）
export CARGO_HOME=E:/ngt-cargo
export CARGO_TARGET_DIR=E:/ngt-tauri-target
export TMP=E:/ngt-tmp
export TEMP=E:/ngt-tmp
export npm_config_cache=E:/ngt-npm-cache

# 步骤 1: 构建桌面端主应用
echo "步骤 1/3: 构建桌面端主应用 (release)..."
cd "D:/用户/16235/Desktop/文档/Agent-Working/Novel_Generate_Team/desktop-tauri"

echo "  - 正在编译 Rust 核心层..."
echo "  - 正在构建前端..."
npm run tauri build -- --no-bundle

# 查找生成的 exe
DESKTOP_EXE="E:/ngt-tauri-target/release/desktop-tauri.exe"

if [ ! -f "$DESKTOP_EXE" ]; then
    echo "错误: 未找到桌面端 exe: $DESKTOP_EXE"
    exit 1
fi

echo "  ✅ 桌面端构建完成"
ls -lh "$DESKTOP_EXE"
echo ""

# 步骤 2: 更新安装器 payload
echo "步骤 2/3: 更新安装器 payload..."
PAYLOAD_DIR="D:/用户/16235/Desktop/文档/Agent-Working/Novel_Generate_Team/installer/src-tauri/payload"

# 备份旧版本
if [ -f "$PAYLOAD_DIR/NovelGenerateTeam.exe" ]; then
    echo "  - 备份旧版本..."
    cp "$PAYLOAD_DIR/NovelGenerateTeam.exe" "$PAYLOAD_DIR/NovelGenerateTeam.exe.bak"
fi

# 复制新版本
echo "  - 复制新构建的主应用到 payload..."
cp "$DESKTOP_EXE" "$PAYLOAD_DIR/NovelGenerateTeam.exe"

echo "  ✅ Payload 更新完成"
ls -lh "$PAYLOAD_DIR/NovelGenerateTeam.exe"
echo ""

# 步骤 3: 构建安装器
echo "步骤 3/3: 构建安装器..."
cd "D:/用户/16235/Desktop/文档/Agent-Working/Novel_Generate_Team/installer"

echo "  - 正在编译安装器..."
npm run tauri build -- --no-bundle

# 查找生成的安装器
INSTALLER_EXE="E:/ngt-tauri-target/release/installer.exe"

if [ ! -f "$INSTALLER_EXE" ]; then
    echo "错误: 未找到安装器 exe: $INSTALLER_EXE"
    exit 1
fi

echo "  ✅ 安装器构建完成"
ls -lh "$INSTALLER_EXE"
echo ""

# 复制到最终输出目录
OUTPUT_DIR="D:/用户/16235/Desktop/文档/Agent-Working/Novel_Generate_Team/installer/dist-out"
mkdir -p "$OUTPUT_DIR"

echo "步骤 4/3: 复制到输出目录..."
FINAL_EXE="$OUTPUT_DIR/NovelGenerateTeam-Setup.exe"
cp "$INSTALLER_EXE" "$FINAL_EXE"

# 自动使用 Qn 证书对生成的安装程序进行数字签名
echo "步骤 5/3: 对安装程序进行数字签名 (署名 Qn)..."
SIGNTOOL_PATH="C:/Program Files (x86)/Windows Kits/10/bin/10.0.19041.0/x64/signtool.exe"
if [ -f "$SIGNTOOL_PATH" ]; then
    "$SIGNTOOL_PATH" sign //sha1 02443A88A5140B0075ADE2C1EDEE728E9100FE32 //tr http://timestamp.digicert.com //td sha256 //fd sha256 "$FINAL_EXE"
    echo "  ✅ 数字签名完成"
else
    # 尝试在 powershell 中执行
    powershell -Command "& 'C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.19041.0\\x64\\signtool.exe' sign /sha1 02443A88A5140B0075ADE2C1EDEE728E9100FE32 /tr http://timestamp.digicert.com /td sha256 /fd sha256 '$FINAL_EXE'"
    echo "  ✅ 数字签名完成 (via PowerShell)"
fi

echo ""
echo "======================================"
echo "✅ 打包与签名完成！"
echo "======================================"
echo ""
echo "安装程序位置:"
echo "  $FINAL_EXE"
echo ""
ls -lh "$FINAL_EXE"
echo ""
echo "可以分发此单文件安装程序给用户使用。"
