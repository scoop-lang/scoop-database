# AGENTS.md

用中文与用户交互。文档使用中文，代码标识符与注释使用英文。

## 项目边界

本仓库维护三个普通 Scoop Cone：`packages/scoop.db`、`packages/scoop.db.sql`、`packages/scoop.db.sqlite`。根 `Cone.toml` 是 workspace；`examples/records` 是 executable 示例成员。三个库全部独立于 Scoop 编译器仓库和 sysroot，库间使用显式相对 path 依赖，系统库使用安装的 Scoop 工具链。

先读 [README](README.md)、[数据库规范](docs/specs/SCOOP-DB-SPEC.md)和受影响库的说明。公开行为或资源合同变更先修改规范，再修改实现、调用方和文档。不要增加 ORM、反射、数据库宏或编译器/runtime 特例来解决普通库问题。

SQLite 使用 `packages/scoop.db.sqlite/vendor/sqlite` 内的官方 amalgamation，随 Cone 编译。不得改回系统 SQLite 包、构建时下载或复制 Scoop 编译器/runtime。上游源码保持原样；升级时同步 vendor 说明、第三方声明、规范和必要测试。

## 验证

完成一批变更后，先执行 `scoop fmt --workspace`、`scoop fmt --workspace --check` 和 `scoop check --workspace`；native bridge 变更还需执行 README 中的 C 严格警告检查，再运行相关库的 `scoop test --release`。涉及库依赖、native 构建或公开接口时运行三个库测试和 `scoop run --release examples/records`。

普通行为测试放在所属 Cone 的 `tests/`，使用 `scoop.test`；不复制一套编译器 fixture runner。用现有真实调用验证，不通过删除或隐藏系统库、源码、工具来测试复用。原始日志放仓库外，文档仅记录验证范围、结果和实际限制。

继承 Scoop 的范围纪律：不新增来源授权、防伪、通用资源预算、重复语义验证或假想插件框架。原生句柄与取消处理只承担必要的生命周期和内存正确性职责。不要提交构建缓存或本机绝对路径。
