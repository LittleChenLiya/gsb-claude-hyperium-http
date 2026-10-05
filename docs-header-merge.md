# HeaderMap 显式合并计划（Explicit Header Merge）

入口在 `http::header` 模块：

- `merge(&a, &b) -> Result<HeaderMap, MergeError>`
- `merge_with_options(&a, &b, &MergeOptions) -> Result<HeaderMap, MergeError>`

这是一个**独立的合并操作**。既有的 `HeaderMap::append` 语义完全不变：它只是往同名
值列表尾部压入一个值，不做任何字段分类或校验。

## 1. 按字段类别处理

| 类别 | 代表字段 | 合并动作 |
| --- | --- | --- |
| list（逗号列表） | `accept`、`vary`、`cache-control`、`www-authenticate` 等 | 两个输入中的**每个值**按出现顺序用 `", "` 拼成**一个**值 |
| 单值（singleton） | `host`、`content-type`、`location`、`date` 等 | 双方值必须一致；重复且相等则折叠成一个；不一致则拒绝合并 |
| `content-length` | `content-length` | 同样要求一致，但冲突时单独报 `ContentLengthConflict` |
| 不可合并多值 | `set-cookie` | **保留每一项**，绝不用逗号拼成一个值 |
| 未知（自定义）字段 | 如 `x-custom` | 保守处理：把多值**逐项保留**，既不逗号拼接，也不判冲突 |

- “多个 Content-Length 必须一致”：去 OWS 后逐字节比较，`42` 与 `42` 折叠为一个，
  `42` 与 `43` 直接拒绝。
- 列表合并在**所有校验通过之后**才进行拼接，原始输入的字节不会被原地修改。

## 2. HeaderName 真实大小写与共享身份

`HeaderMap` 的键本身就是大小写不敏感的，`HeaderName::is_standard()` 用来区分标准头
与自定义头（本次新增的 `pub(crate)` 方法）。因此：

- `Content-Length`、`content-length`、`CONTENT-LENGTH` 是同一个键，不会产生两个键；
- 未知字段也一样：`X-Custom` 与 `x-custom` 合并后仍是一个键下的多个值。

## 3. Connection 提名字段按声明策略过滤

RFC 9110 §7.6.1：`connection` 的值里列出的字段名是“连接特有”的。

- `MergeOptions::default()` / `ConnectionFields::Filter`（默认）：
  - 解析两个输入中所有 `connection` 值的逗号 token（大小写不敏感匹配）；
  - 删除被点名的字段，**并删除 `connection` 字段本身**。
- `ConnectionFields::Keep`：保留 `connection`（按 list 规则合并）及它点名的字段。

## 4. 全通过才产出新 HeaderMap，原输入不改

- 先做完整校验与聚合（只 `clone` 值），中途任何冲突立即返回 `MergeError`；
- 只有全部检查通过，才构造并返回一个**全新**的 `HeaderMap`；
- 失败或成功都不修改两个输入（测试 `merge_returns_new_map_and_never_touches_inputs`
  及冲突用例会快照核对）；
- 输出容量不足时返回 `MergeError::MaxSizeReached`（使用 `try_insert`/`try_append`，
  不 panic）。

## 5. 错误类型

`MergeError`（`#[non_exhaustive]`，实现 `Display`/std `Error`）：

- `ContentLengthConflict { first, other }`
- `SingletonConflict { name, first, other }`
- `MaxSizeReached`

## 6. 五个 Header 的端到端例子

见 `examples/header_merge_five_headers.rs`，覆盖 `accept`（list）、`content-length`
（单值一致）、`set-cookie`（不可合并多值）、`connection`/`keep-alive`（连接特有的
过滤）、`x-custom`（未知保守多值）。

合并前两个块、合并后结果在示例运行时打印。

## 7. 本地 cargo 命令（缓存/产物定向 D 盘）

仓库内带了 `.cargo/config.toml`，把 `CARGO_TARGET_DIR` 与 `TEMP/TMP` 指到
`D:/Codex/gsb-cache/...`；`CARGO_HOME` 也在环境里指向 D 盘缓存。直接运行即可：

```sh
# 只跑本题相关的小范围确定性测试
cargo test --test header_merge

# 本模块文档测试
cargo test --doc merge

# 五个 Header 的可运行例子
cargo run --example header_merge_five_headers

# 回归：既有的 HeaderMap / 常量模式测试 与 库内单测
cargo test --test header_map --test patterns
cargo test --lib
```

不使用 Docker；以上均为本地原生、短命令、小夹具验收。
