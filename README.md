# Scoop Database

Scoop 的数据库访问框架与异步 SQLite 驱动，独立于编译器和 sysroot 维护。仓库是一个包含多个 Cone 的 workspace：

| Cone | 内容 |
| --- | --- |
| [scoop.db](packages/scoop.db/README.md) | 逻辑连接、异步结果流、稳定 Row、普通 record 的双向 codec 映射 |
| [scoop.db.sql](packages/scoop.db.sql/README.md) | SQL 会话、Statement、完整参数绑定、结构化事务 |
| [scoop.db.sqlite](packages/scoop.db.sqlite/README.md) | 内存／文件 SQLite、异步执行、取消、超时和确定性关闭 |
| [records](examples/records/README.md) | record → 事务写入 → 查询 → record 的可运行示例 |

SQLite 3.54.1 官方 amalgamation 随驱动源码编译，**无需安装 SQLite 系统包，构建时不下载依赖**。项目仍需要完整的 Scoop 工具链及目标平台的 C 编译器／SDK。core、std、platform、asyncio 和 test 使用工具链随附的系统库，数据库库之间使用仓库内的 path 依赖。

## 构建与测试

在仓库根目录执行；Windows 的主命令对应 `sco`：

```sh
scoop build --release
scoop test --release
scoop run --release examples/records
```

默认 build/test 选择三个库；`scoop build --release --workspace` 同时构建示例，也可用 `-p scoop:scoop.db.sqlite` 选择单个库。普通构建自动解析本地依赖并生成根目录的 `Cone.lock`；系统库来自当前工具链，无需相邻的 Scoop 源码仓库或向 sysroot 安装数据库库。

示例输出：

```text
7: Ada (no note)
```

格式化和静态检查：

```sh
scoop fmt --workspace
scoop fmt --workspace --check
scoop check --workspace
```

`scoop.db.sqlite` 的 native bridge 使用 C11。修改 bridge 时，从仓库根目录执行严格警告检查（编译器按目标工具链选择）：

```sh
cc -std=c11 -Wall -Wextra -Werror -DSQLITE_THREADSAFE=1 \
  -I packages/scoop.db.sqlite/vendor/sqlite -fsyntax-only \
  packages/scoop.db.sqlite/native/control.c \
  packages/scoop.db.sqlite/native/statements.c
```

上游 SQLite 源码保持原样，不参与项目源码格式化；内嵌版本、许可、编译选项与更新步骤见 [vendor/sqlite](packages/scoop.db.sqlite/vendor/sqlite/README.md)。

## 在应用中使用

当应用与本仓库位于相邻目录时，在应用的 `Cone.toml` 中声明直接使用的库：

```toml
[dependencies]
"scoop:scoop.asyncio" = "0.1.0"
"scoop:scoop.db" = { path = "../scoop-database/packages/scoop.db", version = "0.1.0" }
"scoop:scoop.db.sql" = { path = "../scoop-database/packages/scoop.db.sql", version = "0.1.0" }
"scoop:scoop.db.sqlite" = { path = "../scoop-database/packages/scoop.db.sqlite", version = "0.1.0" }
```

数据库源码推送到本仓库远端后，也可将这三个 path 来源分别换成 Git 来源：

```toml
"scoop:scoop.db" = { github = "scoop-lang/scoop-database", subdir = "packages/scoop.db", version = "0.1.0" }
"scoop:scoop.db.sql" = { github = "scoop-lang/scoop-database", subdir = "packages/scoop.db.sql", version = "0.1.0" }
"scoop:scoop.db.sqlite" = { github = "scoop-lang/scoop-database", subdir = "packages/scoop.db.sqlite", version = "0.1.0" }
```

包管理器将选中的 Git commit 写入应用自己的 `Cone.lock`。库间的相对 path 继续指向同一 checkout 中的其他 Cone。不要把数据库依赖只写成 `"0.1.0"`：`scoop` group 的无来源依赖会查找 sysroot，而数据库库已独立维护。系统库依赖仍使用普通版本声明。

完整程序见 [examples/records/src/main.scoop](examples/records/src/main.scoop)。连接需要活动 EventLoop；用 `useAsync` 在 loop 退出前关闭结果、语句和连接。取消或超时不代表写入已撤销，原子写入使用显式事务。HBase／Cosmos 只用于设计边界讨论，尚无对应驱动。

## 文档与来源

- [数据库规范](docs/specs/SCOOP-DB-SPEC.md)
- [设计与抽象边界](docs/database/DESIGN.md)
- [实施和验证记录](docs/database/PROGRESS.md)

项目提取自 [scoop-lang/scoop PR #121](https://github.com/scoop-lang/scoop/pull/121)，来源提交为 `9f284db52c028709ac772fd672f906b0076e1993`。公开 Cone 坐标和 Scoop API 保持原名，源码、测试、native bridge、文档与示例在本仓库共同维护。

Scoop Database 沿用 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 双许可；内嵌 SQLite 按上游 public domain 声明分发，见 [第三方声明](THIRD-PARTY-NOTICES.md)。
