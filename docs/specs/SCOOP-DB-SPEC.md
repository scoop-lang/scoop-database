# Scoop 数据库基础库规范

本规范规定 `scoop.db`、`scoop.db.sql` 和首个驱动 `scoop.db.sqlite` 的公开行为。具体声明及三种后端的抽象检验见[设计](../database/DESIGN.md)，实施状态见[记录](../database/PROGRESS.md)。这些是普通 Cone，不增加语言、编译器或 runtime ABI。

## 1. 范围与依赖

三个库在独立的 `scoop-database` 仓库中维护，不随 Scoop sysroot 构建或分发。仓库根为普通 Cone workspace，成员位于 `packages/`，库间通过显式相对 path 依赖连接；外部应用通过 path 或 Git/subdir 选择源码。公开坐标与 package 名称继续使用 `scoop:scoop.db`、`scoop:scoop.db.sql`、`scoop:scoop.db.sqlite`，但不能用无来源的版本依赖把它们当作 sysroot 系统库。core、std、platform、asyncio 和测试库仍从用户安装的 Scoop 工具链消费。

`examples/records` 是独立 executable 成员，使用同一组源码库完成 record 往返。项目的 build/test/run 使用现有 Scoop 命令，不复制编译器、runtime 或系统库，不增加构建协议。依赖解析遵守上游[包管理规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-PACKAGE-SPEC.md)。

公共层只规定逻辑 Connection、DataSource、ResultStream、异步生命周期和可选的表格 Row 映射。SQL Statement、参数与事务属于 `scoop.db.sql`；SQLite 原生连接和调用属于 `scoop.db.sqlite`。HBase cells 和 Cosmos 文档不必转换为 Row，本期不实现这两个驱动。

record 是应用的普通 struct/class。表格映射的接口、列注解和值转换均由 `scoop.db.row` 定义。宿主程序 `scoop-db-generator` 使用 Scoop 的 `generator/generate` 协议（版本 2），在编译期为 `@GenerateWith("scoop.db", ["RowWriter<T>", "RowReader<T>"])` 选中的缺失方法生成普通 Scoop 正文。可以分别选择读、写，或手写其中任一方法。调用方通过 `--generator scoop.db=<宿主程序路径>` 注册工具。SQL 文本、事务和参数顺序仍由应用指定。

## 2. 公共资源与异步调用

- Connection 和 ResultStream 实现现有 AsyncCloseable；DataSource.connect 每次交付一个由调用者关闭的逻辑连接。工厂不自动拥有连接。
- 所有数据库 I/O、准备、锁等待、执行、取项、事务完成和资源释放均为 suspend；读取快照与 codec 为普通同步调用。
- 异步入口要求当前 Job 和可用 executor。关闭/回滚允许在已取消 Job 中完成，使用既有 nonCancellableScope。不得用 GC release hook 完成 I/O 或等待。
- 参数与可变容器在提交前取得稳定快照；编码、参数检查、快照失败不发出请求。
- 取消选择和原生工作退休是两件事：原生借用、callback 和未交付资源收尾后才恢复调用者，并恢复到原 execution/executor。排队取消避免开始操作；已运行操作按后端能力请求停止并等待本地真实结束。
- 对只读请求，取消、关闭和成功选择一次结果；先获胜的结果不被后续取消覆盖。可能已提交的写操作交付能够确定的真实成功/失败；超时、断链或取消不证明未写入，不自动重试。
- DbCallOptions.timeout 是每次调用的相对时限，接纳时转换为单调截止时间，包含排队与本次内部续页；不逐页重置。None 不设时限，负值是参数错误，零不提交并立即超时。超时请求停止，仍等待本地退休；不能强制中断同步 codec。
- close 停止接纳，协调在途工作并按拥有关系关闭子资源。并发 close 等同一次收尾；后续重复 close 不重报历史错误。清理不受正文已取消/到期影响，多个失败用 ResourceUseException 保留。
- close 不主动 COMMIT，也不保证撤销写入。原生隐式事务仍按其语义结束。借用的 EventLoop、executor、pool 由外层在数据库资源之后关闭。

DbException 保留 operation、message、cause；Closed/Busy/Timeout 是库能够确定的状态。驱动错误保留原始错误码。取消使用现有 CancellationException；用户 codec/default/constructor 的异常不改写为数据库异常。

## 3. ResultStream

`next(options): T?` 的外层 None 表示结束，Some(None) 可作为可空 T 的数据。同一流最多一个在途 next，竞争时报 DbBusyException；不同流能否并行由专属接口规定。

默认按需求拉取，不自动读完整个查询。允许原生页/批次缓存；空页带续页信息时继续，直到取得一项或明确结束。EOF 退休读取资源，此后 next 继续返回 None；关闭后报 DbClosedException。读取/映射失败使流终止，不暗中重开请求。

在接纳前因参数错误、已取消 Job、零时限或 Busy 拒绝的 next 不改变源流状态；已接纳读取的失败/取消才进入流的失败终态。

交付值在下次 next、挂起和流/连接关闭后仍有效。native 临时指针不能逃逸；不可变存储可以共享。失败 next 在恢复前退休本地读取/游标借用，并使连接恢复明确可用状态或失效。

mapRecords 与 decodeRecords 每次同步映射一项，接管源流的消费和关闭责任。映射失败关闭源流，双失败用 ResourceUseException；调用者不得并行消费已交给 wrapper 的源流。

## 4. Row 与 record 映射

Row/RowSchema 是不可变快照，列和值数量一致，Long 索引从零开始。列名可以重复。按名字读取有歧义时抛 AmbiguousColumnException，缺列时抛 MissingColumnException，getOrNone 仅对缺列返回 None。select 按显式序号投影并重命名，目标名唯一。Column.nativeTypeName 记录后端元数据。RowValue 包含 Null、Boolean、Signed(Long)、Unsigned(ULong)、Float32、Float64、Text、Bytes、Record(Row)、Sequence(Array<RowValue>)。

`Rows.encodeToRow(value, writer)` 调用 `RowWriter<T>.writeRow(value, row: RowBuilder)`。每次 `row.put(name, value)` 添加一个完整 RowValue，按写入顺序建立列，重复名称报 AmbiguousColumnException。`build()` 发布不可变快照。`Rows.decodeFromRow(row, reader, options)` 与 `ResultStream<Row>.decodeRecords(reader, options)` 使用 `RowReader<T>.readRow(row: RowInput)`。RowInput 按列名取 `required` 或 `optional`，返回可复读的 RowCell；重复列应先用 select 消歧。生成代码在构造结果前调用 `finish()`。默认忽略额外列，UnknownColumnPolicy.Reject 检查本层未读取的列，并传递到嵌套行。手写 reader 也可提前调用 `finish()`；直接返回时，读取入口在交付结果前完成该检查。缺必需列、重复列、未知列、值形状和数值范围错误均报 RowMappingException，path 从空串开始，列名中 `~`、`/` 分别转义为 `~0`、`~1`，数组元素使用十进制零基下标。用户转换与构造器异常原样传播。

生成器按主构造器参数顺序映射 struct 或无 class 基类的 final class 的普通存储成员。生成读取要求主构造器是唯一可见构造器。计算属性不占列；有额外存储或自定义访问器时可手写相应方向。`@DbColumn(name = "…", required = …, emitNull = …, ignore = …)` 用于字段或属性，空 name 使用声明名，有效列名须唯一。ignore 与其他设置互斥，忽略的参数使用声明默认值或可空参数的 None。required 要求列出现，emitNull 适用于可空字段。默认省略声明为 Option 的 None；required 或 emitNull 会保留列，内置可空转换将它写作 Null。缺列优先采用原构造参数默认值，其次对可空参数传 None。内置可空转换将显式 Null 读为 None，内置非空转换报告错误。生成代码在转换各列并检查未知列后才执行一次普通具名构造调用，保留默认值的声明作用域、求值顺序与异常。

内置列映射支持 Boolean、各宽度的有符号和无符号整数、Float、Double、String 与 Char（一个 Unicode 标量的 Text）。整数读取可在实际值可表示时跨符号或宽度转换，浮点只额外允许 Float32 提升到 Double。`Array<Byte>` 对应 Bytes（包括空 BLOB），其他 Array 递归映射 Sequence，显式 Null 与空集合各自保留。嵌套 record 通过其对应方向的行映射生成 Record。自定义单列类型可在 companion 实现 `ColumnCodec<T>`，也可在映射器主构造器通过一个匹配的 `val` 传入列转换，后者优先，显式转换自行决定所接收列值的表示。多份匹配的依赖在字段处报错。日期、Decimal、数据库特有的整数布尔值等由显式转换处理。

字段省略只影响行快照。SQL 参数顺序由 `SqlArguments.fromRow` 的 fieldOrder 给出，显式 NULL、缺参数、SQL DEFAULT 与删除分别处理。Record 和 Sequence 保持结构，绑定到 SQL 标量需要应用显式转换。

## 5. SQL 能力

SqlSession.prepare 接收后端方言，不改写通用 SQL。Statement 一次对应一个可执行语句和一个结果，支持完整重新绑定后重复执行。SqlResult 为 Rows(ResultSet) 或 Update(affectedRows?)；Rows 只表示结果形状，不表示只读或已经提交。空查询也有 schema。

execute 启动本次执行；Update 表示执行完成，Rows 后续仍可发生读取/执行失败。ResultSet 的列顺序固定、值类型可变。关闭/耗尽释放会话占用；关闭 Statement 先关闭它的结果，关闭 Connection 清理所有子资源。

SqlArguments 是快照：positional 按公开的零基 slot 顺序，named 按驱动的精确名称。缺少、重复、多余或歧义参数在执行前失败。fromRow 按明确 fieldOrder 选择列，参数类型提示为 None，不把 Column 元数据当写入提示。类型提示由驱动决定，不能拼接 SQL。SQL DEFAULT 由 SQL 文本表达。

`SqlArguments.resolve(parameters: List<SqlParameterInfo>): Array<SqlParameter>` 是驱动的普通公开绑定入口：元数据按连续零基 slot 排列，返回同序快照并检查完整性。named 要求每个 slot 有唯一精确名称；因此首版不接受包含匿名或未命名空隙 slot 的 named 绑定，稀疏 SQLite 参数使用 positional。

首版 SQL 会话单路执行，活动结果占用会话；冲突报 Busy，不隐式等待自己释放结果。事务取得会话执行权，事务外连接和旧 Statement 不得混入；事务中只执行该事务的 Statement。

SqlTransaction.state 为 Active/Committed/RolledBack/Invalid。begin 的迟到成功在未交付时先回滚。commit 只接受 Active 且没有活动结果；可能已经提交后交付实际结果，未知结果使连接失效。确认提交先更新状态，再清理 Statement，清理错误不改写已提交事实。

rollback/close 可在取消后使用；关闭 Active 事务回滚，已完成事务幂等，Invalid 只收尾。后端自动回滚同步到 state。transaction helper 向正文提供 SqlSession，先等待正文结构化子任务，再检查取消并提交；提交后不追加取消检查。原失败与回滚失败均保留。手动事务控制 SQL 不得改变受管理事务的归属。

本版 SQL Row/参数值域不能无损表示的原生类型报转换错误，专属 API 可以保留自己的类型和读取接口；不通过隐式 Text/Double 转换伪装支持。

## 6. SQLite

SqliteDataSource 配置 Memory/File、readOnly、busyTimeout。每次 Memory 连接独立，Memory/readOnly 冲突报参数错误；默认不改变 journal mode 或应用 PRAGMA。busyTimeout 非负，零按原生锁竞争错误返回。

连接借用 connect 时的 EventLoop。所有普通 SQLite 调用在 managed blocking workers 串行执行，worker 可以迁移；定时器和取消只协调停止与恢复。pool/loop 存活到连接关闭。使用随库编译、启用 `SQLITE_THREADSAFE=1` 的 SQLite C API，不新增 runtime 功能。

prepare 由 SQLite 解析并拒绝第二条可执行语句。execute 完成绑定和首次 step，缓存首行或 EOF；next 继续推进，reset/finalize/close 也在 worker 执行。TEXT/BLOB 在交付前形成独立快照；空 BLOB 与 NULL 区分。

NULL/INTEGER/REAL/TEXT/BLOB 分别映射 Null/Signed/Float64/Text/Bytes；TEXT 严格 UTF-8，Boolean 参数为 0/1，Unsigned 超过 Long.MAX_VALUE 时拒绝，Float32 提升为 REAL。SqlParameter.nativeTypeName 必须为 None。

NaN 参数在提交前拒绝，避免 SQLite 将其静默变成 NULL。原生 SQL 改变事务状态而没有匹配的 SqlTransaction 所有者时使连接失效；会话事务通过 beginTransaction 管理。

SQLite 原生 slot 减一作为公开索引，稀疏 ?NNN 按最大 slot 数绑定，具名重复占一个 slot；named 不猜匿名 slot。实际结果类型/列名来自 SQLite，不根据 SQL 前缀猜测。

运行中取消连接 sqlite3_interrupt、progress handler 和可取消 busy 等待。interrupt/close 正确同步，旧 work 与取消通知退休后才允许新操作，空闲 interrupt 不能代替排队/执行前取消检查。busy callback 只读取 native 状态，不执行用户 Scoop 代码。

step 失败后根据原生事务状态同步对象，必要时使连接失效。RETURNING 首次 step 产生修改但不表示提交完成；结束语句可能完成隐式事务，close 不承诺回滚，最终 step/reset/finalize 错误必须交付。

本轮只验证内嵌 SQLite 的本机构建和 Cone 测试，不据此声明跨平台验证或其他数据库支持。

驱动随源码包含 SQLite 3.54.1 官方 amalgamation（`packages/scoop.db.sqlite/vendor/sqlite/sqlite3.c`、配套头文件及上游声明），通过普通 `native.sources` 编译到 Cone。构建不下载 SQLite，不查找或链接系统 `sqlite3` 库；仅使用 Scoop 工具链、目标 C 工具链和平台基础系统库。固定版本和更新步骤见[内嵌源码说明](../../packages/scoop.db.sqlite/vendor/sqlite/README.md)。File 拒绝空路径和裸 `:memory:` 特殊名，内存库使用 Memory。SQLite UpdateResult.affectedRows 为同一串行执行前后的 sqlite3_total_changes64 差值，包含 SQLite 计入的触发器/外键修改，DDL 等无修改语句为零；不把上一次 DML 的 changes 误报为本次结果。
