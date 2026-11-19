# scoop.db

数据库公共资源、异步结果流，以及可选的 `scoop.db.row` 数据与 record 映射。公共 Connection 不假定 SQL、事务、socket 或物理连接；SQL 能力位于 [scoop.db.sql](../scoop.db.sql/README.md)，首个驱动为 [scoop.db.sqlite](../scoop.db.sqlite/README.md)。

`ResultStream<T>.next()` 按需交付稳定值，外层 None 表示 EOF。同一流不允许并发 next；close 停止接纳并等待本地在途工作收尾。通过现有 `useAsync` 管理资源，关闭早于所属 EventLoop/executor 退出。`DbCallOptions.timeout` 作用于单次调用，包含排队；取消和超时不证明写入已回滚。

### 为 record 生成行映射

```scoop
import scoop.db.row.*

struct User(@DbColumn(name = "user_id") val id: Long, val note: String? = None) {
    @GenerateWith("scoop.db", ["RowWriter<User>", "RowReader<User>"])
    public companion object: RowWriter<User>, RowReader<User>
}

val row = Rows.encodeToRow(User(7L), User.Companion)
val user = Rows.decodeFromRow(row, User.Companion)
```

`@GenerateWith` 选择需要的方向，可以只生成读或写。`RowWriter.writeRow` 通过 `RowBuilder.put` 写入完整列值；`RowReader.readRow` 通过 `RowInput.required/optional` 取得 RowCell。生成的 reader 在调用构造器前执行 `finish()`。手写 reader 也可以提前检查，直接返回时由入口完成未知列检查。`ResultStream<Row>.decodeRecords(reader)` 按需映射并接管源流的消费与关闭，映射失败时关闭源流。构建时的宿主程序注册见[仓库说明](../../README.md#构建与测试)。

生成器按构造参数名读取列。`@DbColumn` 支持改名、要求列出现（`required`）、写出可空字段的 `None`（`emitNull`）和忽略（`ignore`）。缺列可触发构造参数默认值，显式 Null 交给字段映射。使用 `RowDecodingOptions(UnknownColumnPolicy.Reject)` 可以拒绝未读取的列。遇到重名列时，先用 `Row.select` 按序号投影并重命名。

`Array<Byte>` 写成 BLOB，其他 Array 写成 Sequence，嵌套 record 写成 Record。将嵌套值绑定到 SQL 标量需要显式转换。整数读取检查范围，Float 可以提升为 Double。自定义单列类型可在 companion 实现 `ColumnCodec<T>.toValue/fromValue`，或把 `ColumnCodec<T>` 作为映射器的主构造 `val` 传入。RowCell 的类型错误携带完整列路径。

完整规则见[数据库规范](../../docs/specs/SCOOP-DB-SPEC.md)与[设计](../../docs/database/DESIGN.md)。使用示例见 [SQLite record 往返](../../examples/records/README.md)。
