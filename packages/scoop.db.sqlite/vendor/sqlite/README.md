# 内嵌 SQLite

本目录保存 SQLite **3.54.1** 官方 amalgamation 中原样提取的 `sqlite3.c`、`sqlite3.h` 和 `sqlite3ext.h`。不构建或分发上游 CLI `shell.c`。

- 来源：[sqlite-amalgamation-3540100.zip](https://www.sqlite.org/2026/sqlite-amalgamation-3540100.zip)
- 上游[下载页](https://www.sqlite.org/download.html)公布的归档 SHA3-256：`92281c86e2957f9b9305890f9ac04204caa7b677cc868dc326d942689df28e30`
- 导入日期：2026-11-17。
- 许可：上游 [public domain 声明](https://www.sqlite.org/copyright.html)；保留源码文件中的原始声明。

`scoop.db.sqlite/Cone.toml` 将 `sqlite3.c` 作为普通 native source，与 bridge 一起编译进入库产物；头文件目录为 `vendor/sqlite`，使用 C11 和 `SQLITE_THREADSAFE=1`。连接继续以 `SQLITE_OPEN_FULLMUTEX` 打开。其余 SQLite 编译选项使用上游默认值；未额外启用 FTS 或其他可选扩展。

不调用 pkg-config，不查找 `libsqlite3`，不在构建时联网。Linux 声明 SQLite 实际需要的平台基础库 `pthread`、`dl` 和 `m`；Darwin／Windows 使用各自工具链的基础系统库。它们不是外置 SQLite 开发包。上游源码不套用 bridge 的 `-Wextra -Werror` 检查或项目格式化，bridge 单独执行 README 中的严格检查。

升级时从官方获取对应的 amalgamation，核对一次下载页公布的归档摘要，替换这三个原始文件，并同步本说明、项目 README、第三方声明和数据库规范中的版本。随后运行三个 Cone 的 release 测试与 record 示例，检查最终可执行文件未链接系统 SQLite。归档校验仅用于维护时下载，不进入日常构建或缓存规则。
