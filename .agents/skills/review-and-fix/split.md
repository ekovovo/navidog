# 只搬代码的拆分

把一个大文件拆成 `<名字>/mod.rs` 加若干子模块，行为不变。完成标准：`verify_move.py` 只报出被 rustfmt 折行的签名；全量检查通过；单独一个提交。

## 步骤

1. **看结构**（在仓库根目录运行）：

   ```bash
   rg -n "^(pub(\(crate\))? )?(struct|enum|impl|fn|mod|const|type)|^    (pub(\(crate\)|\(super\))? )?fn " <文件>
   ```

2. **分组**：按功能把方法分到子模块（例如表单视图、查找替换、单元格编辑器、右键菜单、就地编辑）。父模块要构造或读字段的类型留在父模块；测试里用 `super::x` 引用的函数也留在父模块。
3. **写 spec 并运行**：脚本把文件 `git mv` 成 `<名字>/mod.rs`，把列出的方法从 `impl <type>` 块搬进子模块并改成 `pub(super)`，把 `items` 里的顶层代码原样搬走，再在父模块里声明子模块：

   ```bash
   python3 .cursor/skills/review-and-fix/scripts/split_impl.py /tmp/split.json
   ```

   ```json
   {
     "source": "crates/navidog-grid/src/viewer.rs",
     "type": "TableViewer",
     "modules": {
       "find": {"doc": "Find and replace on the loaded page (F-DATA-007).",
                "methods": ["open_find", "find_step", "render_find"]},
       "fk_picker": {"doc": "Foreign key data selection (F-DATA-012).",
                     "methods": ["open_fk_picker"],
                     "items": [["struct FkPicker {", "}"], ["impl FkPicker {", "}"]]}
     }
   }
   ```

   `items` 的两个标记都是整行原文，从第一行起搬到第一个等于结束标记的行为止，上方紧挨着的 `///` 和 `#[` 一起搬。同一个起始标记出现多次时只认第一个。

4. **另一种类型的整段代码**（结构体、它的固有 `impl`、`trait impl` 连在一起，例如 `GridDelegate`）：用一小段 Python 直接剪到子模块，再把它的非 pub 字段和固有方法改成 `pub(super)`（`trait impl` 里的方法不加可见性），父模块加 `use 子模块::类型;`。
5. `cargo fmt --all`，再跑这个 crate 的 clippy。报“cannot find type”就在父模块补 `use 子模块::类型;`。
6. **核对只是搬动**：

   ```bash
   git show HEAD:<原文件> > /tmp/old.rs
   python3 .cursor/skills/review-and-fix/scripts/verify_move.py /tmp/old.rs <新目录> <类型> [<类型> …]
   ```

   两侧都为空，或只剩加了 `pub(super)` 后被 rustfmt 折成多行的签名，才算通过。
7. 跑全量检查，单独提交 `refactor: split <什么> into modules, moving code only`。属于某次审查的，在 CR 文件日志里记下行数变化和核对结果。
