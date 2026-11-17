# 数据库框架实施记录

来源：[scoop PR #121](https://github.com/scoop-lang/scoop/pull/121)，提取提交 `9f284db52c028709ac772fd672f906b0076e1993`（原分支 `codex/database-sqlite`）。以下实现与验证记录来自原 PR；独立项目的迁移与复测记录另列。范围：公共资源与 Row/codec、SQL 能力、SQLite 完整异步驱动；HBase/Cosmos 保持设计检验。

## 批次与状态

1. 设计与规范：已建立设计、库规范和本记录。
2. 公共层与 Row/codec：已实现 `scoop.db` 的公共声明、稳定 Row/RowSchema、精确标量与 binary codec、Encoder/Decoder、惰性 record 映射和关闭协调。
3. SQL 接口与参数/事务组合：已实现 `scoop.db.sql` 的会话、Statement/ResultSet、参数快照与 resolve、事务接口和结构化 helper；事务执行行为已通过真实 SQLite 测试。
4. SQLite 原生驱动、完整异步、取消和资源生命周期：已实现 `scoop.db.sqlite`，覆盖内存/文件/只读、单语句准备与重复绑定、逐项读取、取消/超时、事务与确定性关闭；复用 managed blocking workers 和 EventLoop。
5. SQLite record 闭环、关键竞态、示例和收尾：已验证 record → 参数 → 数据库 → Row → record，交付[可运行示例](../../examples/records/README.md)及三层库说明。原 PR 已构建本机预编译库；独立项目改由普通源码依赖构建，不再安装到 sysroot。

每个实现批次只运行相关 Cone 的 release 测试；重复验证针对新变化或未解决问题。临时日志保存在仓库外，构建缓存保留供后续复用。

## 原 PR 验证记录

- 文档批次：本地链接、章节编号、代码围栏和 whitespace 检查通过；没有实现或运行验证。
- 公共层批次：本机 aarch64-apple-darwin release 的 `scoop.db` 16 项 Cone 测试通过，覆盖嵌套/缺失/NULL/二进制/数值、encoder 提交与 decoder 复读、映射异常和并发 next/close。使用现有工具和 sysroot，仅显式指定本机已安装的 LLD；未重建工具链或运行跨平台测试。
- SQL 能力批次：同一本机 release 的 `scoop.db.sql` 5 项参数测试通过，覆盖位置/名称/完整性、record 显式字段顺序、缺失与 NULL、原生类型提示边界。公共层未变化，复用其已验证产物。
- SQLite 批次：本机 release 的 22 项 Cone 测试全部通过。覆盖文件重连/只读、内存隔离、完整绑定、稳定值/空 BLOB/非法 UTF-8、RETURNING、事务提交/正文失败/取消回滚/自动回滚、会话独占、运行与排队取消、超时、锁等待、关闭在途读取，以及已确认写入的结果和 executor 恢复。
- 收尾竞态：真实文件锁阻塞 SQLite finalize 时，并发 ResultSet.close 等待 native 收尾；修复了过早解除 Statement 资源归属的问题。旧取消退休后连续查询可复用连接，自动回滚后的旧 Statement 立即失效。
- 示例以三个生产 `.slib` 编译运行，输出 `7: Ada (no note)`。合计 43 项本地 Cone 测试通过；C 编译使用 `-std=c11 -Wall -Wextra -Werror`，Scoop 格式与文档链接检查通过。一次非法 UTF-8 测试的异常类型断言已按语言规范修正并复测闭合。

## 原 PR 的范围限制

原 PR 仅验证 aarch64-apple-darwin 和本机 SQLite 3.54.0；未运行跨平台、全仓或 GC stress 验收，不声明 HBase/Cosmos 驱动可用。当时使用系统 SQLite 3.37+ 开发库；独立项目已改为下述内嵌源码构建。

## 独立项目迁移（2026-11-17）

- 三个库及其所有原有测试和 native bridge 完整迁至 `packages/`；原有生产实现和测试正文未改动。新仓库使用普通 Cone workspace，库间改为显式相对 path，系统库消费已安装工具链，不引用相邻 Scoop 源码仓库。
- record 示例迁为 `examples/records` executable Cone，补充 package 声明及源码依赖，根目录提供统一 build/test/run 入口、许可和维护说明。规范、设计和所有本地文档链接已同步。
- 内嵌 SQLite 3.54.1 官方 amalgamation，保留原始 C 源码、头文件与声明；通过 `native.sources` 编译并启用 `SQLITE_THREADSAFE=1`，移除系统 `sqlite3` 链接。新增一项真实 SQL 测试核对内嵌版本与线程安全配置；不增加 compiler/runtime ABI。
- 使用已安装 Scoop `0.0.0-dev.20261115`（`6bd92d3d7a14e4614320008b33586df8da178d85`），在 aarch64-apple-darwin 上验证。Scoop workspace 格式检查、静态检查、C bridge 的 C11／`-Wall -Wextra -Werror`、文档本地链接和项目 whitespace 检查通过。
- `scoop build --release --workspace` 构建全部三个库及示例；`scoop test --release` 的 16 + 5 + 23 项测试全部通过，共 44 项。`scoop run --release examples/records` 输出 `7: Ada (no note)`。
- 示例和 SQLite 测试可执行文件的 `otool -L` 仅列出 `libSystem.B.dylib`，没有系统 SQLite；`nm` 确认两个程序都定义了 `sqlite3_open_v2`、`sqlite3_libversion` 和 `sqlite3_threadsafe`。未删除或隐藏任何工具、源码或系统库。
- 仓库外临时应用通过相对 path 直接依赖三个库，使用安装工具链完成 `check --offline` 与 `run --release --offline`，得到相同 record 往返输出；验证普通下游消费不依赖 workspace 成员身份。该应用及其日志未纳入项目源码。

本次未进行 Linux／Windows 运行验收或 GC stress；不将本机结果表述为跨平台通过。仓库只提供 SQLite 驱动，HBase／Cosmos 仍是设计讨论。
