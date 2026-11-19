# SQLite record 往返示例

使用已安装的 Scoop 工具链和目标 C 编译器／SDK；SQLite 源码随驱动编译，无需安装系统 SQLite 开发包。项目准备见[仓库说明](../../README.md)。

在仓库根目录构建生成器并运行示例。

```sh
cargo build --locked --release --manifest-path generator/Cargo.toml
scoop run --release --generator scoop.db=generator/target/release/scoop-db-generator examples/records
```

这是普通 executable Cone，清单通过 path 依赖消费三个源码库。进入示例目录运行时，可将生成器路径改为 `../../generator/target/release/scoop-db-generator`，省略末尾的示例目录。

[src/main.scoop](src/main.scoop) 创建独立内存库，在事务中写入普通 struct，再把查询 ResultSet 惰性映射为同一个类型，输出 `7: Ada (no note)`。所有资源在 EventLoop 退出前通过 useAsync 关闭，不创建文件。

record companion 用 `@GenerateWith("scoop.db", …)` 生成 `RowWriter<User>` 和 `RowReader<User>`。`@DbColumn(emitNull = true)` 使 note=None 写成显式 SQL NULL，满足第三个绑定参数。查询按另一种顺序返回列，读取器按名字还原 User。
