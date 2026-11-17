# SQLite record 往返示例

使用已安装的 Scoop 工具链和目标 C 编译器／SDK；SQLite 源码随驱动编译，无需安装系统 SQLite 开发包。项目准备见[仓库说明](../../README.md)。

在仓库根目录执行：

```sh
scoop run --release examples/records
```

也可进入本目录执行 `scoop run --release`。这是普通 executable Cone，清单通过 path 依赖消费三个源码库，不需要预先把数据库 `.slib` 放入 sysroot。

[src/main.scoop](src/main.scoop) 创建独立内存库，在事务中写入普通 struct，再把查询 ResultSet 惰性映射为同一个类型，输出 `7: Ada (no note)`。所有资源在 EventLoop 退出前通过 useAsync 关闭，不创建文件。

record companion 复用语言已有的 ValueEncoder/ValueDecoder 合成。`emitNull = true` 使 note=None 编码为显式 NULL，以满足三个 SQL 参数；查询故意使用不同列顺序，验证按名字解码。SQL 和字段顺序由程序给出，不生成 ORM 查询。
