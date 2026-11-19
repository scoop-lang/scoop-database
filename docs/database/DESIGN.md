# Scoop 数据库访问与 record 映射设计

原设计日期：2026-11-10；独立项目拆分：2026-11-17。状态：公共层、SQL 能力与 SQLite 首版已完成本机实现和验证；公开合同见[库规范](../specs/SCOOP-DB-SPEC.md)，实际完成项见[实施记录](PROGRESS.md)。

SQLite 3.54.1 官方 amalgamation 随驱动源码编译，启用线程安全，不依赖系统 SQLite 开发包；来源与构建选项见[内嵌源码说明](../../packages/scoop.db.sqlite/vendor/sqlite/README.md)。

本项目独立于 Scoop 编译器仓库，三个已实现的库位于 `packages/`，通过普通 path/Git 依赖构建，使用已安装的 Scoop 系统库；构建和示例入口见[项目说明](../../README.md)。

本文确定库的接口和语义。SQLite 是首个实现；HBase 与 Cosmos DB for NoSQL 用于检验抽象，不表示承诺在同一轮实现这两个驱动。本文中的 Scoop 代码块是公开声明或使用示意，实际实现和验证范围由实施记录说明。

## 1. 目标与依据

提供基础数据库访问、完整异步生命周期，以及数据库数据与普通 Scoop record 的映射。这里的 record 是应用的 struct/class，不是共同基类、持久化实体或具有运行时身份的框架对象。

设计遵守以下已有合同：

- [语言规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-SPEC.md) 3.2、9.5 的普通泛型接口与 `@GenerateWith`，宿主进程按[生成器协议](https://github.com/scoop-lang/scoop/blob/main/docs/milestone47/PROTOCOL.md)查询类型和成员。
- 同一规范 11.9.1～11.9.3、11.18：Job、取消、executor、blocking、确定性资源释放。
- [异步 I/O 规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-ASYNCIO-SPEC.md)第 3 节：结果选择与原生请求结束分开协调，AsyncCloseable/useAsync。
- [并发库规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-CONCURRENT-SPEC.md)第 5 节：managed BlockingTaskPool 与执行登记的退出义务。
- [运行时规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-RUNTIME-SPEC.md) 3.4～3.8、4.5：原生借用、线程状态和 release hook 的限制。
- [编译器与产物规范](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-IMPL-SPEC.md) 2.7～2.8：普通 Cone、native 输入和链接；数据库不形成新的 stage、实体角色或 ABI。
- [公共 API 风格](https://github.com/scoop-lang/scoop/blob/main/docs/specs/SCOOP-API-STYLE.md)：Long 索引、Option、资源命名、快照和显式 codec。

本期不设计 ORM、表模型、关系加载、变更跟踪、自动增删改、schema 迁移、SQL 构造 DSL 或编译期数据库查询。不增加反射、数据库宏或数据库专属方法合成。record 的行读写既可由数据库生成器生成，也可由应用手写。数据库区分整行映射和单列转换，可与同一模型上独立声明的 JSON 映射共存。

## 2. 分层与共同边界

| 计划中的 Cone | 内容 | 依赖方向 |
| --- | --- | --- |
| `scoop.db` | Connection、DataSource、ResultStream、调用与关闭合同；可选使用的 Row 数据模型和 row codec 适配 | core、std、asyncio |
| `scoop.db.sql` | SQL 会话、Statement、表格 ResultSet、参数绑定、SQL 事务能力 | db |
| `scoop.db.sqlite` | SQLite 配置、SQL 能力实现、native 调用与调度 | db、db.sql、platform、asyncio |
| `scoop.db.hbase` | 本文检验用的 Get/Scan/Put、cell 与 record 映射 | db；实际传输依赖在接入时确定 |
| `scoop.db.cosmos` | 本文检验用的文档、点操作、分页与分区批次 | db、json；实际 HTTP 客户端接入在实现时确定 |

`scoop.db` 内的 `scoop.db.row` package 是表格数据工具，不是另一套必经流水线。SQL 驱动使用它；文档驱动可以直接使用 JsonValue；HBase 保留自己的 cell 集合。所有数据库不必先转换成 Row 或共同的值树。

| 问题 | 公共层决定 | 专属接口决定 |
| --- | --- | --- |
| 访问对象 | 资源归属、关闭与在途工作收尾 | 物理连接、客户端、连接池或远端会话 |
| 请求 | 不规定共同的 `execute(String)` | SQL 文本与绑定、Get/Scan、文档点操作、作业提交 |
| 结果 | 有类型的异步结果流；稳定的数据交付 | 行、文档、cells、分页、原生批次 |
| 数据映射 | 表格按列读写，其他后端选择自己的数据映射 | 字段地址、二进制表示、版本选择、专属标量 |
| 事务 | 不要求每个 Connection 支持事务 | SQL 会话事务、分区原子批次、单行原子操作 |
| 并发 | 单个流的消费规则与关闭协调 | 同一客户端可否并行、会话独占范围 |
| 取消 | 本地收尾、结果竞争、恢复位置 | 是否能取消远端操作及如何得知最终结果 |

能力通过普通接口和返回的具体类型表达。例如 SQLite 连接实现 TransactionalSqlConnection，HBase 连接提供 table，Cosmos 连接提供 container。不存在返回 Any 的 `capability(name)`、通用能力注册表，或要求驱动为不适用的方法提供失败 stub 的总接口。

Data Warehouse 也遵守这个边界：支持 SQL 不表示必须维护长连接，查询可以返回作业句柄后再读取结果。列式 batch 可由专属 API 保留，按行视图按需适配；不要求先物化整个结果。本文不预建作业或列式 SDK。

## 3. 最小公共接口

下列声明位于 `scoop.db`，AsyncCloseable 来自现有 scoop.asyncio。

```scoop
public interface Connection : AsyncCloseable {
    public val isClosed: Boolean
}

public interface DataSource<C : Connection> {
    public suspend fun connect(options: DbCallOptions = DbCallOptions()): C
}

public interface ResultStream<T> : AsyncCloseable {
    public val isClosed: Boolean
    public suspend fun next(options: DbCallOptions = DbCallOptions()): T?
}

public struct DbCallOptions(val timeout: Duration? = None)

public open class DbException public constructor(
    public val operation: String,
    message: String,
    public val cause: Throwable? = None
) : Exception(Some(message))

public class DbClosedException public constructor(operation: String)
    : DbException(operation, "database resource is closed")
public class DbBusyException public constructor(operation: String)
    : DbException(operation, "database resource is busy")
public class DbTimeoutException public constructor(operation: String)
    : DbException(operation, "database operation timed out")
```

Connection 是逻辑访问资源，不承诺一条 socket、单线程、自动提交或事务能力。`isClosed` 在关闭停止接纳新操作时变为 true；关闭工作可能仍在推进，须等待 close 返回才表示本地收尾完成。

DataSource 是显式的配置和连接工厂，不自动成为连接的 owner。每次成功 connect 交付一个由调用方关闭的逻辑 Connection。首版 SQLite DataSource 不持有后台资源；未来实际持有池的实现须另实现 AsyncCloseable 并说明池的归属，不能从此工厂接口推导池生命周期。工厂和配置不在全局注册或自动发现。

错误分类只表达库能确知的事实。驱动异常继承 DbException，保留该数据库的原始 code、status 或其他必要字段。SQLSTATE 只属于有此概念的 SQL 驱动，不在公共异常中伪造。参数与索引错误使用既有参数异常；codec/default/constructor 的异常原样传播。Job 取消继续使用已有 CancellationException，不统一包装为 DbException。

### 3.1 ResultStream 的状态与数据所有权

| 状态 | next | close |
| --- | --- | --- |
| 可读且没有在途 next | 异步取得下一项或确认结束 | 开始关闭 |
| 已有在途 next | 抛 DbBusyException | 停止该次读取并等本地收尾 |
| 已正常耗尽 | 返回 None；仍检查调用取消 | 幂等收尾 |
| 已接纳的读取失败或 next 被取消 | 后续 next 报状态错误；不得暗中重新开始查询 | 收尾 |
| 正在关闭或已关闭 | 抛 DbClosedException | 参与同一次关闭或立即返回 |

一次读取失败会使这个流进入终态，但不自动推导所属连接失效。失败/取消的 next 在恢复调用者前退休本次读取和流持有的本地游标借用，停止继续取项；驱动把连接恢复到明确可用状态，或关闭/标记失效。不能交付仍在读取旧响应的连接，也不等待无法确认的远端停止；后续 close 只完成剩余收尾。

接纳前的参数错误、已取消 Job、零时限或 Busy 拒绝不消费、不终止源流；该规则与已接纳请求的失败收尾分开。

`next(): T?` 的外层 None 只表示结束。T 自身为 Option 时，Some(None) 仍是一项有效结果。空分页不是结束；实现继续取得后续页，直到有项目或后端明确结束，期间遵守同一次 next 的取消和截止时间。

交付的数据在后续 next、挂起、流关闭和连接关闭后仍然有效。驱动可以共享不可变页/批次存储；依赖 native 临时指针的数据必须先复制或取得独立、稳定的存储所有权。record 中可变引用的后续同步仍由应用负责。

每个流同时最多一个 next。不同流是否可以并行由所属专属接口规定；共同 Connection 不施加串行队列。默认按消费需求拉取，允许为了协议页/批次而缓存已取得的项目，不在消费者停止后无界读取剩余结果。不会把整个结果自动转换为 Array。

流在返回最后的 None 前释放自己的数据请求和读取资源，仍可保留只读元数据。显式 close 不要求消费剩余数据，不主动提交显式事务或取消独立远端作业；它也不保证已发生的写入被撤销。结束写语句可能触发后端自身的隐式事务完成，见第 7 节。

## 4. 完整异步合同

### 4.1 调用与调度

所有可能进行数据库 I/O、等待锁/worker、取消原生请求或释放数据库资源的入口均为 suspend。读取已取得的 Row/文档/cells、取得本地视图和执行 codec 是同步操作。

异步操作要求当前 Job 和可用的 CoroutineExecutor；缺失时在提交前报 IllegalStateException。普通请求检查 Job 取消，关闭/回滚等清理按第 4.4 节处理。网络及定时器使用现有 EventLoop。首版 SQLite 的 connect 在活动 EventLoop 中建立连接并借用该 loop 的定时服务；之后可由其他 executor 调用，但所属 loop 必须保持运行直到连接关闭。具体网络驱动同样说明其驱动归属，不使用隐藏的全局线程或事件循环。

恢复回到调用者的 execution/executor，worker 不直接运行其 continuation frame。内部状态锁只保护接纳、选择结果、占用关系和取消登记；不跨 I/O、用户 codec、回调或挂起持有。

数据库库可以使用既有 blocking 调度协议，但不能仅在同步函数外加 suspend 就宣称支持运行中取消。现有 blocking 在工作开始后交付真实结果；数据库操作须额外连接自己的取消机制，或明确等待无法中断的工作结束。所有 work 的执行登记保留到真实 runner 退出，包含 caller 已取消的情形。

### 4.2 输入快照与提交边界

SQL、参数列表、路径、文档、cell 列表和字节输入在请求交给 worker/网络前形成稳定快照。不可变对象可共享，List/MutableArray 等可能改变的容器须复制必要结构。输入求值、codec 编码和快照构造失败时不提交数据库请求。

“已提交”是可能产生数据库可见效果的边界，不是创建了一个本地请求对象。驱动以自己的调用或传输边界决定这一事实；边界不明时按可能已提交处理。该状态只服务当前操作，不形成凭证、持久收据或跨阶段证明。

### 4.3 结果选择、取消与超时

必须分开协调两个事实：调用者将收到什么结果，以及本次本地/native 工作何时不再访问资源。只能恢复 continuation 一次。

| 时点/操作 | 行为 |
| --- | --- |
| 登记前已经取消 | 不提交，抛 CancellationException |
| 仍排队且取消先获胜 | 不执行请求；撤销或退休队列项 |
| connect/prepare/只读请求正在执行 | 取消和成功交付竞争；取消获胜时清理未交付句柄/数据，再恢复取消 |
| 写操作已可能提交 | 尝试驱动提供的取消；保留已取得的真实成功/失败，不把取消解释为撤销 |
| 已选定成功，之后取消 | 交付成功；后续独立取消检查仍能观察 Job 取消 |
| 关闭/回滚等必要清理 | 调用者取消不提前结束清理 |

对于已提交写操作，能够取得确定结果时必须交付这个结果，即使等待期间发生 Job 取消或到达时限。取消/超时在此阶段首先是停止请求，不能抢先覆盖随后取得的确定成功或数据库失败。因中断返回的原生错误可保存在驱动异常中；连接丢失或超时使结果无法确认时，报告对应失败并明确该路径可能已经产生效果。CancellationException/DbTimeoutException 本身也不证明“没有写入”。公共层不重试写请求。SQL commit 的处理见第 6.4 节。

取消通知只发送停止请求或改变本地选择；不会在取消 handler 中同步等待 worker、驱动网络或执行用户代码。本地原生工作仍在访问资源时，取消等待不能提前释放 buffer、句柄、callback 或其保活引用。native callback 返回、借用解除和未交付资源清理完成后，才恢复调用者。

远端请求可能在本地传输结束后继续运行。公共 close/取消保证本地收尾；远端 job 的取消、查询状态及写入结果由专属接口表达。不能为了等待不可能确认的远端终止而永久保留已无用途的本地借用，也不能把断开网络表述为远端回滚。

DbCallOptions.timeout 为一次调用的相对时限：None 表示不设本地时限；负值在提交前报参数错误；零在取消/参数检查后立即超时且不提交。正值在接纳调用时转换为单调截止时间，覆盖排队、I/O 和该次调用内部的后续页/重试，不在每页重新计时。

timeout 到期使用与取消相同的收尾路径。排队/只读请求由超时先获胜时返回 DbTimeoutException；已提交写入仍按上一段交付能够确定的实际结果，无法确认时才报告超时或相应驱动失败。先已选定的结果保持。deadline 是请求停止的时点，不承诺原生不可中断调用已经结束，因此返回可能晚于时限。next 的 timeout 只覆盖此次取项，不覆盖消费者处理记录的时间；整个查询的总时限可以由应用所属 Job 控制。同步 codec 不被强制中断。

定时任务与主操作属于同一结构化生命周期。主操作完成后取消并退休自己的定时任务；清理不再沿用已经到期的正文时限。有限 timeout 依赖实际驱动借用的 EventLoop，不新增通用 timer/runtime 接口。

### 4.4 关闭与作用域退出

1. close 原子停止接纳新工作；尚未选定结果的只读请求以 DbClosedException 结束，已经获胜的取消/结果保持。可能已提交的写操作继续遵守第 4.3 节，不以关闭冒充写入失败。
2. 取消/退休排队项，等在途 native work、借用和 callback 退出，继续处理必要的协议收尾。
3. 按拥有关系关闭子资源，再释放自身资源。某一步失败仍继续其他独立清理。
4. 多个并发 close 等待同一次收尾并观察其结果；收尾后再次 close 是无操作，不重复抛旧错误。

close 在已取消 Job 中仍可调用，内部使用 nonCancellableScope 完成必要工作。useAsync 保留正文与清理异常；多个清理失败按清理次序用 ResourceUseException 嵌套保存，不吞掉后续失败。driver 不主动销毁仍被已交付值引用的不可变存储。

本地对象的 close 不主动执行 COMMIT，不自动重试未知结果，不等待自己所在 worker 退出。隐式事务遵守后端结束语句的实际行为，不能把关闭当作回滚。借用的 executor、EventLoop、HTTP client 或 blocking pool 由外层所有者在数据库资源关闭后回收；数据库对象不关闭借用设施。自有设施的收尾必须在依赖它的 work 全部结束后执行。

GC release hook 不承担取消、回滚、网络访问、排空或线程 join。这里的完整异步包括打开、准备、执行、取项、事务完成、取消和关闭，而不仅是查询入口。

## 5. Row 与 record 映射

### 5.1 可选的公共 Row 表示

下列声明位于 `scoop.db.row`。Row 表示一组有序列值；不推导数据库表、主键或更新目标。

```scoop
public struct Column(val name: String, val nativeTypeName: String? = None)

public class RowSchema internal constructor() {
    public val columns: Array<Column>
    public fun indexOf(name: String): Long?
    public companion object {
        public fun fromColumns(columns: List<Column>): RowSchema
    }
}

public enum RowValue {
    Null
    Boolean(Boolean)
    Signed(Long)
    Unsigned(ULong)
    Float32(Float)
    Float64(Double)
    Text(String)
    Bytes(Array<Byte>)
    Record(Row)
    Sequence(Array<RowValue>)
}

public class Row internal constructor() {
    public val schema: RowSchema
    public fun get(index: Long): RowValue
    public fun get(name: String): RowValue
    public fun getOrNone(name: String): RowValue?
    public fun select(columns: List<ColumnSelection>): Row
    public companion object {
        public fun fromValues(schema: RowSchema, values: List<RowValue>): Row
    }
}

public struct ColumnSelection(val name: String, val index: Long)
public class MissingColumnException public constructor(name: String)
    : Exception(Some("missing column: " + name))
public class AmbiguousColumnException public constructor(name: String)
    : Exception(Some("ambiguous column: " + name))
```

工厂复制 List 结构，Row/RowSchema 和载荷数组只读，内部不可变子值可以共享。fromValues 要求列数与值数一致，失败不发布 Row。公共 RowValue 是这项工具支持的值域，不是所有数据库必须使用的 wire 类型系统。专属值仍可留在驱动原生结果中。

序号从零开始，类型为 Long。schema 允许重复列名；indexOf 在没有匹配时返回 None、多个匹配时报 AmbiguousColumnException。get(name) 在没有匹配时抛 MissingColumnException；getOrNone 仅缺列时返回 None，显式 Null 返回 Some(RowValue.Null)。索引越界使用 IndexOutOfBoundsException。

nativeTypeName 是数据库确实提供的类型名快照；未知为 None。它不决定列值的运行时种类，不是由列名猜出的 schema，也不代表 NULL 的可接受性。SQLite 动态值类型可以随行变化。

select 按明确源序号投影并重命名，不执行 I/O，保留对应原生类型元数据并共享不可变值。目标名称须唯一。这个入口解决 join 的重名列与查询别名适配；不遍历 record 的类型描述来猜映射。

### 5.2 生成整行读写方法

```scoop
public interface RowWriter<T> {
    public fun writeRow(value: T, row: RowBuilder)
}
public interface RowReader<T> {
    public fun readRow(row: RowInput): T
}
public interface ColumnCodec<T> {
    public fun toValue(value: T): RowValue
    public fun fromValue(cell: RowCell): T
}
```

`scoop-db-generator` 从待实现接口取得 record 的完整类型，按编译器返回的主构造器与存储成员关系生成普通 Scoop 代码。生成的 writer 对每个选中字段调用 `row.put(列名, 完整值)`。reader 按列名选择 `required` 或 `optional`，转换当前行后调用 `finish()`，再用具名实参构造结果。缺列时省略有默认值的实参，让原构造器负责求值。类、字段、构造器和自定义转换仍通过普通源码访问规则绑定。

`RowBuilder.build()` 发布快照。同名写入报错。RowSchema 保存可复用的列名索引及首个歧义列，多个查询行可以共享 schema。RowInput 用它按名字读取稳定 RowCell。默认忽略未读取列；严格模式在 `finish()` 检查未读取的列，检查成功后释放访问标记。手写 reader 直接返回时由入口完成检查。重复名称用 `select` 按序号消歧。嵌套记录和数组的错误路径继承父字段，按 JSON Pointer 转义。

### 5.3 单列转换

`RowCell(row.get("id")).readLong()` 可以独立读取一列。整数按实际值检查符号和宽度，Float32 可提升为 Float64。Boolean 只接受 RowValue.Boolean。SQLite 的 INTEGER 布尔值可用应用提供的 `ColumnCodec<Boolean>` 转换，分别检查 0 和 1。类型错误报含列路径的 RowMappingException。

`Array<Byte>` 对应原生 Bytes，空 BLOB 与 Null 可区分。其他 Array 映射为 Sequence；嵌套 record 的 companion 提供对应方向的行映射。日期、Decimal 等值的一列表示由 `ColumnCodec<T>` 决定。生成器先查映射器主构造器上匹配的 `val`，再用内置转换或字段类型的 companion。完整的用户读写方法也可直接构造和检查 RowValue。

### 5.4 字段存在性和写入意图

`@DbColumn` 管理数据库列名、必需列、显式 Null 和忽略字段。缺列在读取时可触发原默认值；显式 Null 传给可空字段。默认省略可空成员的 None；`required` 或 `emitNull` 将其写成有名称的 Null 列。SQL NULL、未绑定参数、SQL DEFAULT 和删除分别由各自入口表达。嵌套值持久化为 JSON 文本等单列类型时由应用显式转换。

### 5.5 流式 record 读取

```scoop
struct User(val id: Long, val name: String, @DbColumn(emitNull = true) val note: String?) {
    @GenerateWith("scoop.db", ["RowWriter<User>", "RowReader<User>"])
    public companion object: RowWriter<User>, RowReader<User>
}

val row = Rows.encodeToRow(User(7L, "Ada", None), User.Companion)
val restored = Rows.decodeFromRow(row, User.Companion)
```

`ResultStream<Row>.decodeRecords(reader, options)` 通过 `mapRecords` 每次转换一个 Row。wrapper 接管源流的消费和关闭，映射失败时关闭源，正文和清理双失败由 ResourceUseException 保留。读取使用此次 next 的时限，已交付 record 保持有效。完整事务与查询调用见[示例](../../examples/records/src/main.scoop)。

## 6. SQL 能力层

### 6.1 会话、语句与结果

```scoop
public interface SqlSession {
    public suspend fun prepare(
        sql: String,
        options: DbCallOptions = DbCallOptions()
    ): Statement
}

public interface SqlConnection : Connection, SqlSession

public interface TransactionalSqlConnection : SqlConnection {
    public suspend fun beginTransaction(
        options: DbCallOptions = DbCallOptions()
    ): SqlTransaction
}

public struct SqlParameterInfo(val index: Long, val name: String?)

public interface Statement : AsyncCloseable {
    public val isClosed: Boolean
    public val parameters: Array<SqlParameterInfo>
    public suspend fun execute(
        arguments: SqlArguments,
        options: DbCallOptions = DbCallOptions()
    ): SqlResult
}

public interface ResultSet : ResultStream<Row> {
    public val schema: RowSchema
}

public enum SqlResult {
    Rows(ResultSet)
    Update(UpdateResult)
}

public struct UpdateResult(val affectedRows: Long?)
```

这些声明位于 scoop.db.sql，不改变 Connection。prepare 接收具体数据库方言，不解析或重写成通用 SQL。首版一次 Statement 对应一条可执行语句和一个结果；多语句脚本、多结果过程不属于这个接口合同。

这个能力承诺本版 RowValue/SqlParameter 能表示的表格值域，不承诺覆盖所有数据库原生类型。后端类型无法在此值域中无损交付时，通用入口明确报转换错误；保留原生 Decimal 等能力的专属查询可以直接返回自己的结果与读取接口。不通过 Text/Double 的隐式降级伪装成通用支持，也不要求所有支持 SQL 的后端都实现 SqlConnection。

Statement 可以重复执行，但同时只有一次执行或一个活动 ResultSet；重叠调用报 DbBusyException，不自动关闭旧结果。每次参数完整传入，执行前形成快照，不保留上次绑定。无参数语句使用 SqlArguments.empty()。

execute 根据实际结果种类返回 Rows/Update，不能按 SQL 文本前缀猜测。空查询仍返回具有 schema 的 ResultSet；返回数据的写操作同样使用 Rows。affectedRows 为 None 表示后端没有提供可靠计数；零是确知的零，不使用负数哨兵。各数据库的受影响行计数口径由驱动注明。

execute 成功表示已经启动本次原生执行，不把执行整体推迟到第一次 next。Update 表示语句执行结束；Rows 表示取得结果流，后续仍可能发生执行/读取失败。

Rows 只描述结果形状，不保证只读或写入已经完成。带 RETURNING 的写语句在 execute/next 中遵守写操作取消规则，直到原生执行实际结束；一次 next 成功也不等于显式事务提交。原生操作性质不明时按可能写入处理。提前关闭、读取失败或 codec 失败均不承诺撤销先前写入；需要整组操作原子性时由应用使用明确的事务。

ResultSet 的 schema 在交付时可读，同一次结果内列顺序固定；列值的种类可以不同。ResultSet 关闭释放这次执行，Statement 可再次使用；Statement 关闭先关闭其结果。Connection 关闭处理仍拥有的 statements、streams 和 transactions。已交付 Row 不在这条关闭链上。

### 6.2 参数与 record 编码的衔接

```scoop
public struct SqlParameter(val value: RowValue, val nativeTypeName: String? = None)
public struct NamedSqlParameter(val name: String, val parameter: SqlParameter)

public class SqlArguments internal constructor() {
    public fun resolve(parameters: List<SqlParameterInfo>): Array<SqlParameter>
    public companion object {
        public fun empty(): SqlArguments
        public fun positional(values: List<SqlParameter>): SqlArguments
        public fun named(values: List<NamedSqlParameter>): SqlArguments
        public fun fromRow(row: Row, fieldOrder: List<String>): SqlArguments
    }
}
```

工厂发布不可变快照。positional 使用公开的零基 slot 顺序；named 使用驱动报告的精确参数名，包含方言要求的前缀，不统一删除 `:/$/@`。缺少参数、重复名称、多余参数或无法消歧的名字，在执行数据库语句前报参数错误。

resolve 供独立驱动 Cone 按参数元数据取得完整绑定快照，不需要跨 Cone 访问私有字段。元数据须按连续零基 slot 排列；named 要求每个 slot 有唯一精确名称，包括 SQLite 稀疏编号留下的 slot，含未命名空隙时使用 positional。

fromRow 按 fieldOrder 显式选择列并形成 positional 参数；每个指定字段必须存在且唯一。Row 多余字段不参与绑定。生成参数的 nativeTypeName 为 None，不把读取时的 Column 元数据变成写入提示。它不使用 struct 声明序猜 SQL 占位符，也不生成 SQL。显式 SqlParameter.nativeTypeName 是后端类型提示，主要用于类型推断不足的参数；驱动按支持的绑定类型解释，不拼接到 SQL 中。不接受的提示在执行前失败。

Record/Sequence 不作为首版 SQL 通用参数；使用它们必须显式选择列 codec。SQL DEFAULT 由 SQL 文本表达，不伪装成 NULL 参数。

```scoop
connection.prepare("INSERT INTO users(id, name) VALUES (?, ?)").useAsync { statement ->
    val row = Rows.encodeToRow(User(7L, "Ada", None), User.Companion)
    val fieldOrder: Array<String> = ["id", "name"]
    val arguments = SqlArguments.fromRow(row, fieldOrder)
    statement.execute(arguments)
}
```

上例只是接口用法示意；调用方需处于已建立的协程和数据库驱动作用域。SQL 的目标与字段选择由应用明确给出。

### 6.3 SQL 会话并发

首版 SQL 能力采用单路会话：同一 Connection 同时最多一个执行操作；活动 ResultSet 直到耗尽或关闭前占用其会话。冲突调用报 DbBusyException，不排入一个可能等待自己关闭结果的隐藏队列。需要并行时使用多个 Connection。

事务取得整个会话的执行权。事务期间经原 Connection 或它之前创建的 Statement 发起执行会报状态错误；事务内只能使用该 SqlTransaction 创建的 Statement。事务拥有的 Statement 仍遵守单路结果规则，事务结束后失效。

这些约束只属于 SQL 能力，本设计的 HBase/Cosmos 客户端可以并行提交独立请求。关闭与所有普通操作始终正确同步。

### 6.4 SQL 事务

```scoop
public enum SqlTransactionState { Active, Committed, RolledBack, Invalid }

public interface SqlTransaction : SqlSession, AsyncCloseable {
    public val state: SqlTransactionState
    public suspend fun commit(options: DbCallOptions = DbCallOptions())
    public suspend fun rollback()
}

public suspend fun <T> TransactionalSqlConnection.transaction(
    block: suspend (SqlSession) -> T
): T
```

事务对象在成功 begin 后交付。begin 期间取消且数据库实际已经开启事务时，先回滚未交付的事务。无法恢复明确会话状态则关闭连接。事务不支持隐式嵌套；savepoint 以后通过具体能力单独提供，不对所有数据库虚构嵌套事务。

commit 要求活动结果已经关闭；否则报 DbBusyException 并保持 Active。提交前检查取消；一旦 COMMIT 可能已经提交，等待并交付确定结果，不能因随后取消把已确认成功改成取消。无法确定提交结果时进入 Invalid，连接不得复用，报告驱动异常；不自动重试 COMMIT，也不声称回滚成功。

commit 只接受 Active，其他状态报 IllegalStateException。确认提交成功先将 state 设为 Committed，再完成事务拥有的 Statement 清理；清理失败也不能改写已经提交的事实。确认回滚后设为 RolledBack，无法恢复明确状态则为 Invalid。结束后的事务不能再 prepare/execute，状态读取仍可用于判断真实结果。

rollback/close 允许在已取消 Job 中执行。rollback 先关闭事务子资源，再结束仍活动的事务；已 Committed/RolledBack 时无操作。close 对 Active 执行 rollback，对 Invalid 只完成必要的失效资源收尾。原生错误导致事务已经回滚时同步 state，不重复假设其仍活动。

transaction 是普通泛型函数，只向 block 暴露 SqlSession，由函数负责事务结束；需要手动 commit/rollback 时使用 beginTransaction。它使用 coroutineScope 等待 block 的结构化子任务结束，再检查取消并提交。正文失败/提交前取消则在 nonCancellableScope 回滚；清理错误与原失败组合。提交失败但后端明确事务仍 Active 时尝试回滚；结果未知时按 Invalid 处理。T 中逃逸的事务资源会在作用域退出时失效，稳定 Row/record 可以返回。

用于等待 block 的 coroutineScope 在开始 commit 前已经结束；确认提交后不追加会把成功改成取消的检查。这样 helper 与直接调用 commit 使用同一结果合同，不在作用域出口隐藏已确认的提交结果。

应用不得在受 SqlTransaction 管理的会话中自行执行 BEGIN/COMMIT/ROLLBACK 等改变该事务归属的原生 SQL。驱动不因此增加通用 SQL parser；发现原生事务状态已与对象不符时使对象失效并报告错误，不猜测提交结果。

## 7. SQLite：首个落地检验

```scoop
public enum SqliteLocation {
    Memory
    File(Path)
}

public struct SqliteOptions(
    val readOnly: Boolean = false,
    val busyTimeout: Duration = Duration.ZERO
)

public class SqliteDataSource public constructor(
    location: SqliteLocation,
    configuration: SqliteOptions = SqliteOptions()
) : DataSource<SqliteConnection> {
    public override suspend fun connect(
        options: DbCallOptions = DbCallOptions()
    ): SqliteConnection
}

public class SqliteConnection internal constructor() : TransactionalSqlConnection
```

Memory 的每次 connect 创建独立数据库，不伪装成跨连接共享内存库。File 使用实际平台路径，拒绝空路径和裸 `:memory:` 特殊名；readOnly 与 Memory 冲突时在打开前报参数错误。默认不改变 journal mode、持久化级别或执行应用 PRAGMA。busyTimeout 必须非负，零表示遇到锁竞争直接按原生错误返回。

实现采用以下策略；实际验证范围见[实施记录](PROGRESS.md)：

- 使用随库包含的 SQLite 3.54.1 官方 amalgamation，经现有 native 构建/链接流程编入 Cone；启用线程安全，不下载或链接系统 SQLite。连接操作在 managed blocking workers 执行；事件循环只协调任务、定时器和完成。
- 一条连接的普通 native 调用串行，worker 可以迁移；不要求每个连接常驻一个专用线程。使用支持线程安全的 SQLite 构建；跨线程 interrupt 是专门协调的控制操作。
- 准备、step、reset/finalize、rollback/close 都在工作路径完成，不把收尾留在 EventLoop 同步执行。
- execute 完成绑定并进行首次 step；有结果列时把首行或 EOF 状态保留给 ResultSet，没有结果列时等待语句结束。首行最多暂存一项；返回零行的查询仍交付 schema。
- Row 交付前取得独立字符串/字节数据；SQLite 临时 column 指针不跨 step 或挂起逃逸。参数使用稳定副本或明确的同步借用。
- SQLite 的 NULL/INTEGER/REAL/TEXT/BLOB 分别交付 Null/Signed/Float64/Text/Bytes；严格处理文本，不能把非法 UTF-8 替换后冒充原值。Boolean 参数写为 0/1，Unsigned 只有能精确表示为有符号 64 位时接受；Float32 可精确提升为 SQLite REAL。NaN 参数在提交前拒绝，避免被 SQLite 静默写成 NULL。其他形状由显式 codec 转换或在执行前拒绝。
- SQLite 参数使用其动态绑定规则，nativeTypeName 必须为 None；非空提示在执行前拒绝，不模拟强制参数类型。这不影响 Column 中实际可得的声明类型元数据。
- 参数 slot 按 SQLite 的原生序号减一公开。`?NNN` 的稀疏 slot 按最大 slot 数形成 positional 输入，未引用 slot 的值不产生效果；重复具名参数共享一个 slot。named 模式要求所有 slot 都有名字，不推测匿名或空隙 slot 名称。
- prepare 通过 SQLite 的实际解析结果检查尾部，接受尾随空白/注释，拒绝第二条可执行语句；不自写 SQL 分词框架。
- UpdateResult.affectedRows 使用本次串行执行前后的 sqlite3_total_changes64 差值，包含 SQLite 计入的触发器和外键修改。DDL 等无修改语句为零，不沿用上一条 DML 的计数；口径见 [SQLite total_changes](https://www.sqlite.org/c3ref/total_changes.html)。

SqliteException 继承 DbException，保留 `extendedCode: Int` 和低八位 `primaryCode: Int`。原生 SQL 使会话进入没有对应 SqlTransaction 的事务，或改变受管理事务的归属时，连接失效并要求 close；不猜测事务对象。自动回滚更新 transaction.state，原事务 Statement 同时停止接纳操作。

运行中取消结合 sqlite3_interrupt、progress handler 和可取消的 busy 等待。interrupt 必须绑定当前操作；它返回前禁止连接 close，旧操作和它的取消通知退休前禁止下一操作开始，避免误伤下一条语句。取消发生在 native step 尚未开始时，也必须由排队检查/progress 状态观察，不能依赖一次空闲 interrupt。

busy handler 只能检查稳定的 native 取消/截止状态并按允许的等待规则返回，不修改连接、不调用用户 Scoop 代码。busyTimeout 与调用 deadline 独立；若两者同时存在，以实际先触发者决定停止原因。

写操作被中断可能使整个 SQLite 事务回滚。工作返回后按实际 autocommit/事务状态更新 SqlTransaction，失败时不保留假的 Active 状态。close 等所有 statements 与 interrupt 调用结束后释放连接。上述约束来自 SQLite 的实际取消和句柄生命周期，[官方 interrupt 合同](https://www.sqlite.org/c3ref/interrupt.html)、[busy handler](https://www.sqlite.org/c3ref/busy_handler.html)、[事务状态](https://www.sqlite.org/c3ref/get_autocommit.html)。

RETURNING 的修改在首次 step 中发生，交付首行并不表示提交已经完成。reset/finalize 结束语句可能完成 SQLite 自己的隐式事务；close 不能承诺回滚。驱动保留最终 step/reset/finalize 的失败，应用要撤销整组写入须显式使用 SqlTransaction。这里沿用[SQLite RETURNING](https://www.sqlite.org/lang_returning.html)和[隐式事务](https://www.sqlite.org/lang_transaction.html)的语义。

SQLite 验证公共层的异步、快照与映射能力；它的串行模型、SQL NULL、INTEGER 范围和事务模型不提升为所有数据库的规则。

## 8. HBase：保留 cell 模型的检验

本节定义用于检验边界的最小 Scoop 接口形状，不承诺完整 HBase SDK、传输协议或本期实现。

```scoop
public struct CellAddress(val family: Array<Byte>, val qualifier: Array<Byte>)
public struct HBaseCell(val address: CellAddress, val timestamp: Long, val value: Array<Byte>)
public struct HBaseRow(val key: Array<Byte>, val cells: Array<HBaseCell>)

public enum HBaseColumns {
    All
    Selected(Array<CellAddress>)
}
public enum HBaseVersions {
    Latest
    UpTo(Long)
}
public struct HBaseGet(
    val key: Array<Byte>,
    val columns: HBaseColumns,
    val versions: HBaseVersions = HBaseVersions.Latest
)
public struct HBaseScan(
    val startInclusive: Array<Byte>?,
    val endExclusive: Array<Byte>?,
    val columns: HBaseColumns,
    val versions: HBaseVersions = HBaseVersions.Latest
)
public struct HBaseCellWrite(
    val address: CellAddress,
    val value: Array<Byte>,
    val timestamp: Long? = None
)
public struct HBasePut(val key: Array<Byte>, val cells: Array<HBaseCellWrite>)

public interface HBaseTable {
    public suspend fun get(request: HBaseGet, options: DbCallOptions = DbCallOptions()): HBaseRow
    public suspend fun scan(request: HBaseScan, options: DbCallOptions = DbCallOptions()): ResultStream<HBaseRow>
    public suspend fun put(request: HBasePut, options: DbCallOptions = DbCallOptions())
}
public interface HBaseConnection : Connection {
    public fun table(name: String): HBaseTable
}
```

table 只建立借用客户端的本地句柄，不执行 I/O；Connection 关闭使其失效。独立 get/put/scan 可以并发，每个扫描流仍只有一个在途 next。数组为稳定快照，列族/限定符/row key 均保留 bytes，不要求 UTF-8。

get 返回请求选中的 cells；空集合只表示本次投影没有数据，不能据此断言逻辑 row 不存在。Selected 要求非空且地址不重复，UpTo 的数量必须为正，scan 边界使用 HBase 原生字节排序。扫描的原生 partial result 在驱动中组成完整的 HBaseRow 后再交付；原生 batch 细节不让调用方误以为同一个 row key 是多个业务 record。

HBase 原始结果保存所有返回的版本及地址，不按相同字符串字段名覆盖。原生时间戳单位和有效范围由 HBase 接口规定，不解释为数据库无关的时间。Put 的 timestamp=None 使用后端赋值语义；返回只表示对应原生操作已确认，超时/断链不证明未写入。

### 8.1 显式 cell 与 record 映射

```scoop
public enum HBaseCellFormat { Bytes, Utf8, Int64BigEndian }
public struct HBaseField(
    val field: String,
    val address: CellAddress,
    val format: HBaseCellFormat
)
public enum HBaseReadVersion {
    Latest
    At(Long)
}
public struct HBaseRecordMapping(
    val fields: Array<HBaseField>,
    val readVersion: HBaseReadVersion = HBaseReadVersion.Latest
)
```

此处仅描述 cell 映射的输入。字段名关联实际地址，format 决定字节解释。Bytes 保留原始字节，Utf8 严格验证，Int64BigEndian 要求恰好八字节。写目标地址必须唯一，未选的 cells 留在原始结果中。未来的 HBase 驱动可直接按自己的 cell 表示读写 record。

Latest 明确选择已返回版本中的最大 timestamp，At 要求指定版本；找不到该版本表现为缺字段。原始 HBaseRow 不因此丢弃其他版本。应用要求处理全部版本时直接使用原始 cells，不通过这个单值 record 映射。

encodeToPut 按字段映射生成单次 Put，每个值使用后端时间戳，readVersion 不用于写入。row key 由参数显式给出，不从名为 id 的属性推断。省略的字段不产生 cell write。显式 Null 是映射错误，空字节仍是有效值。删除旧 cell 必须使用明确的 HBase 删除操作，不由 None 推导。向已有 row 写部分字段不承诺清除未出现的旧字段。

该设计保留 HBase 自身的数据模型。更多 cell 格式由后续真实使用扩展，不建立注册表或任意运行时插件。HBase 的 Result 确实保留 cell 的 family、qualifier、timestamp 与 bytes，参见[官方实现](https://github.com/apache/hbase/blob/master/hbase-client/src/main/java/org/apache/hadoop/hbase/client/Result.java)。

## 9. Cosmos DB for NoSQL：文档与分页检验

本节只讨论 Cosmos DB for NoSQL API；其他 API 不自动共用其模型。文档使用已有不可变 JsonValue，复用 Json.encodeToValue/decodeFromValue，不转成 Row。

```scoop
public enum CosmosKeyComponent {
    Missing
    Null
    Boolean(Boolean)
    Number(Double)
    Text(String)
}
public class CosmosPartitionKey internal constructor() {
    public val components: Array<CosmosKeyComponent>
    public companion object {
        public fun fromComponents(values: List<CosmosKeyComponent>): CosmosPartitionKey
    }
}
public struct CosmosItemKey(val id: String, val partitionKey: CosmosPartitionKey)
public struct CosmosItem(val body: JsonValue, val etag: String)
public struct CosmosQueryParameter(val name: String, val value: JsonValue)
public struct CosmosQuery(val text: String, val parameters: Array<CosmosQueryParameter>)
public struct CosmosContinuation(val value: String)
public struct CosmosPage<T>(val items: Array<T>, val continuation: CosmosContinuation?)

public interface CosmosContainer {
    public suspend fun read(key: CosmosItemKey, options: DbCallOptions = DbCallOptions()): CosmosItem?
    public suspend fun create(
        partitionKey: CosmosPartitionKey,
        document: JsonValue,
        options: DbCallOptions = DbCallOptions()
    ): CosmosItem
    public suspend fun replace(
        key: CosmosItemKey,
        document: JsonValue,
        ifMatch: String? = None,
        options: DbCallOptions = DbCallOptions()
    ): CosmosItem
    public suspend fun readPage(
        query: CosmosQuery,
        partitionKey: CosmosPartitionKey?,
        continuation: CosmosContinuation?,
        options: DbCallOptions = DbCallOptions()
    ): CosmosPage<JsonValue>
    public fun query(query: CosmosQuery, partitionKey: CosmosPartitionKey?): ResultStream<JsonValue>
}
public interface CosmosConnection : Connection {
    public fun container(database: String, name: String): CosmosContainer
}
```

container/query 仅建立本地句柄/惰性流，网络工作发生在 read/create/replace/readPage/next。一个客户端可并行处理独立请求；资源与连接关闭仍正确协调。partitionKey=None 的查询表示请求跨分区，不等于分区键本身是 Null。fromComponents 复制非空组件列表并拒绝非有限 Number；组件保留 Missing/Null，实际组件数量及分区路径由容器合同约束，不从 record 字段名猜测。

read 的 None 只表示点读取的不存在结果；其他失败保留 status/substatus。write 的 document 必须是对象且满足请求的显式 id/partition 条件；本地已知不一致在发送前拒绝，其余由服务端按实际容器规则判断。replace 是整个文档替换，省略字段可能删除旧属性；不得把它解释为与 HBase Put 相同的部分合并。ifMatch 保留 ETag 的并发前置条件语义，不变成通用行版本号。

query 返回值可能是文档、数组或标量，因而 ResultStream<JsonValue> 不要求每项都有 id/ETag。点读取的 CosmosItem 则单独保留 ETag。业务解码直接调用 Json.decodeFromValue(item.body, codec)，嵌套 record/sequence 保持已有 JSON 语义；系统字段按 codec 的未知字段规则处理。

readPage 保留后端 continuation 原值，不解析、签名或生成另一套恢复证明。调用方负责将其用于对应容器、查询、参数和分区选择，后端拒绝时正常报错。流式 query 内部消费页，空页仍有 continuation 时继续；一次 next 的 deadline 不因换页重置。

显式分页以整页为交付边界，continuation 指向该页之后；消费者只处理半页就保存该 token 会跳过剩余项目。流不提供“任意单项之后可恢复”的虚假承诺，取消后的新查询也不保证 exactly-once 或同一数据快照。上述分页行为见[官方分页文档](https://learn.microsoft.com/cosmos-db/query/pagination)。

### 9.1 分区原子批次不是会话事务

```scoop
public enum CosmosBatchOperation {
    Create(JsonValue)
    Replace(String, JsonValue, String?)
    Delete(String, String?)
}
public struct CosmosBatchItemResult(
    val status: Int,
    val subStatus: Int?,
    val body: JsonValue?
)
public struct CosmosBatchResult(
    val isCommitted: Boolean,
    val operations: Array<CosmosBatchItemResult>
)
public interface CosmosTransactionalContainer : CosmosContainer {
    public suspend fun executeBatch(
        partitionKey: CosmosPartitionKey,
        operations: List<CosmosBatchOperation>,
        options: DbCallOptions = DbCallOptions()
    ): CosmosBatchResult
}
```

Replace/Delete 的 String 为显式 item id，最后的 String? 为 ifMatch。批次固定一个容器和一个分区键；提交前完成全部输入编码和快照。返回保留各项原生状态，isCommitted 只在整个批次成功时为 true；各项数量与请求顺序对应。传输丢失导致无法取得结果时抛驱动异常，不伪造一个“全部失败、没有效果”的 BatchResult。

这个接口不提供 begin/commit/rollback，也不允许在多次挂起调用间积累一个已经开启的服务端事务。失败项/依赖失败的状态来自真实响应，不能把每一项独立重试。事务范围见[官方 transactional batch 文档](https://learn.microsoft.com/en-us/azure/cosmos-db/transactional-batch)。

## 10. 抽象检验与计划验收

### 10.1 三个后端的对应关系

| 维度 | SQLite | HBase | Cosmos DB for NoSQL |
| --- | --- | --- | --- |
| 逻辑访问对象 | 原生连接的异步 owner | 集群客户端及 table 句柄 | 客户端及 container 句柄 |
| 请求 | SQL Statement + SqlArguments | Get/Scan/Put | 点操作、CosmosQuery、分区批次 |
| 输出 | ResultSet（Row 流）或 UpdateResult | HBaseRow/cells 流 | CosmosItem、JsonValue 流/页 |
| record 读取 | Rows.decodeFromRow | 显式地址、格式与版本映射 | Json.decodeFromValue |
| record 写入 | encodeToRow 后明确绑定字段顺序 | encodeToPut，key 显式传入 | Json.encodeToValue 后明确选择 create/replace |
| 空值 | SQL NULL | 无通用 NULL；缺 cell 与空 bytes 分开 | 缺属性与 JSON null 分开 |
| 并发 | 单路会话/活动结果占用 | 独立请求可并行 | 独立请求可并行 |
| 原子性 | SQL 会话事务 | 原生单行操作语义 | 同容器、同分区批次 |
| 取消 | native 中断 + 等待 worker 收尾 | 取消本地 RPC/扫描，不伪造远端撤销 | 停止本地等待/后续页，不伪造写入撤销 |

三个模型共用结果流和资源生命周期，record 映射由各后端的数据形状决定。SQLite 用于核对调用路径与原生 API 的对应关系；HBase 检查二进制地址与多版本，Cosmos 检查嵌套文档、分页和受限事务。此处为设计审查，不是运行验证。

### 10.2 同一个 record 的具体映射案例

```scoop
public struct Profile(val name: String, val score: Long, val note: String?) {
    @GenerateWith("scoop.db", ["RowWriter<Profile>", "RowReader<Profile>"])
    public companion object: RowWriter<Profile>, RowReader<Profile>
}

val profile = Profile("Ada", 7L, None)
```

| 后端 | 输入数据与映射 | 解码结果 / 再编码行为 |
| --- | --- | --- |
| SQLite | 查询返回 `name=TEXT("Ada")`、`score=INTEGER(7)`、`note=NULL`；驱动形成 Text、Signed、Null 三列 | Rows.decodeFromRow 得到 profile；encodeToRow 默认省略 note。两个参数的 INSERT 可以只绑定 name/score；三个参数的 INSERT 必须显式选择 emitNull 或补充 Null，不能把缺参数自动当作 NULL |
| HBase | 显式映射 name→`info:name`/Utf8、score→`info:score`/Int64BigEndian、note→`info:note`/Utf8；score 返回两个版本，最新 bytes 为 `00 00 00 00 00 00 00 07`，note 无 cell | Latest 解码得到 profile；原始 HBaseRow 仍有两个版本。encodeToPut 使用外部给定 key，生成 name/score 两个 cell；已有 note 不会因此删除 |
| Cosmos | 点读取 body 为 `{"id":"p7","tenant":"t1","name":"Ada","score":7,"note":null}`，容器分区路径为 `/tenant` | 另行提供 JSON 映射后，Json.decodeFromValue 可按 JSON 的未知字段规则忽略 id/tenant，得到 profile。编码 profile 后，应用显式合入 id/tenant 并提供分区键；用 replace 写回时，省略 note 会移除旧属性 |

三个例子使用同一种业务 record。行映射、cell 映射和文档映射分别选择。SQLite 参数顺序、HBase 地址/字节格式/row key、Cosmos 身份/分区条件分别由调用方给出；普通 record 本身不承担这些身份含义。字段 codec 若采用不同的二进制、时间或数值表示，应显式选择对应策略，不承诺任意 codec 可无条件跨后端使用。

另有三个必须保留的边界案例：SQLite join 的两列都叫 id 时，按列序号 select 并重命名后再解码；HBase 请求两个版本时，不能把它们覆盖成一个 String→Value map；Cosmos 查询返回空页但带 continuation 时，下一次页请求继续，同一 next 最终交付数据或明确 EOF。Cosmos 标量投影可以直接交给标量 JSON codec，不必包装成只有一列的 Row。

### 10.3 公共层与映射测试

后续实施的普通行为测试使用所属 Cone 的 scoop.test：

1. struct/class 生成的行映射的字段、重命名、默认值、Option 和显式依赖组合；手写 codec/constructor 的异常保留。
2. 缺列、显式 Null、空文本、空 bytes、空序列的区别；重复列及显式序号投影；整数边界、精度和错误 path。
3. Row 编码字段省略后，参数绑定缺值失败；显式 Null 成功；声明序与 SQL 参数序不同仍按显式 fieldOrder 绑定。
4. 取得 Row/record 后继续 next、关闭流及连接，已交付值仍有效；RowCell 复读不消费源流。
5. mapRecords 惰性、转换失败关闭源、双失败保留；两个 next 竞争只接纳一个，EOF 与 close 行为明确。

测试可以使用最小的受控测试实现来制造异步竞争，不为测试增加生产工厂或通用注入框架。

### 10.4 异步与 SQLite 验收

- 真实长查询期间，EventLoop 的其他任务继续推进；open/prepare/step/finalize/rollback/close 均检查实际运行线程和调度。
- 排队前取消、排队取消、运行中取消、超时、完成后取消、取消与 close 并发、迟到 open/prepare 成功；每次只交付一次，资源在真实结束后释放。
- 旧取消不能 interrupt 下一条语句；取消与 native close 竞争不访问已释放句柄；busy 等待可取消且 deadline 不重置。
- 空结果、逐项读取、提前结束、参数重复执行、缺参数及空 BLOB；RETURNING 归类为 Rows。
- 正常提交、正文失败、取消回滚、native 自动回滚、commit 成功后取消、清理失败；事务外操作不能混入活动事务。
- 连接/流所有者在 executor/loop 之前关闭；借用池不被数据库对象关闭；关闭后没有遗留 work、callback 或借用。

HBase/Cosmos 在本轮只进行接口和数据案例审查：多版本 cell 不丢失、NULL/缺失语义不同、空分页不结束、文档嵌套不扁平化、分区批次不冒充 SQL 事务。真正接入这些驱动时，必须增加真实服务的对应验收，不能以 SQLite 或测试替身宣布后端已支持。

原生 ABI/链接、进程边界或必须由外部进程控制的行为使用少量专门 fixture；普通库行为不复制成多套 fixture。GC stress 只用于跨 worker/native 借用的定向正确性测试。原始日志保存在仓库外，文档只保留结果概要。

## 11. 实施顺序与规范影响

设计阶段交付本设计和路线图入口；后续实施状态与验证结果独立记录，不以接口示意代表已实现。

后续实施按以下顺序形成可用闭环：

1. 将已经评审的公共/SQL 合同落入数据库库规范，明确稳定值、取消和参数规则；生成方法使用公开 generator 协议。
2. 实现 Row 数据、Rows 编码/解码和明确的绑定规则，完成普通 Cone 测试。
3. 接入公共资源/结果流、SQLite 的完整异步调用、取消、关闭与事务；形成 record → 参数 → 执行 → Row → record 的真实闭环。
4. 完成本机关键竞态验收和使用示例，按独立 workspace 构建库，并由示例 Cone 消费源码依赖。本轮按任务范围仅用本机 scoop.test 验证，不运行跨平台验收。
5. 独立接入 HBase/Cosmos 时，以实际需求细化其专属协议；公共层只有出现真实共同语义时才扩展。

不要求新增语言语法、编译器角色、IR/meta、runtime ABI 或跨数据库 query AST。若实现发现现有基础能力不足，应先明确具体问题并修订对应规范；不能把实现便利升级为数据库通用机制。

## 12. 外部参考

以下一手资料于 2026-11-10 核对；它们用于判断真实边界，不构成 Scoop API 的兼容承诺。

- [Swift Codable](https://developer.apple.com/documentation/foundation/encoding-and-decoding-custom-types)：类型与格式侧 Encoder/Decoder 的分工。
- [SQLKit SQLRowDecoder](https://github.com/vapor/sql-kit/blob/main/Sources/SQLKit/Rows/SQLRowDecoder.swift)、[GRDB record](https://github.com/groue/GRDB.swift#record-protocols-overview)：行到普通业务类型的适配，与单列表示分开。
- [SQLite statement](https://www.sqlite.org/c3ref/stmt.html)、[参数绑定](https://www.sqlite.org/c3ref/bind_blob.html)、[线程模式](https://www.sqlite.org/threadsafe.html)：首个驱动的实际能力。
- [HBase AsyncTable](https://github.com/apache/hbase/blob/master/hbase-client/src/main/java/org/apache/hadoop/hbase/client/AsyncTable.java)、[Result](https://github.com/apache/hbase/blob/master/hbase-client/src/main/java/org/apache/hadoop/hbase/client/Result.java)：异步表操作和多版本 cell 结果。
- [Cosmos 分页](https://learn.microsoft.com/cosmos-db/query/pagination)、[分区事务批次](https://learn.microsoft.com/en-us/azure/cosmos-db/transactional-batch)：分页与事务范围。
- [BigQuery 异步作业](https://docs.cloud.google.com/bigquery/docs/running-jobs)：SQL 语言与作业/会话模型是不同维度。
