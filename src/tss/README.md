This directory maintains all things related to syntax parsing and syntax
highlighting.

Presently `Ted` uses [tree-sitter][tree-sitter] for parsing plain-text to
syntax tree. For each parser a subdirectory to be created under ``src/tss/``,
it is also allowed to maintain the original parser repo else-where and
manage it via ``git-submodules``. Similarly, for each parser highlighting
rules can be provided in ``.tss`` format and placed under ``src/tss/``
directory. For example, author all ``toml`` related grammars under
``src/tss/toml/`` directory and author syntax-highlighting rules for
``toml`` under ``src/tss/toml.tss`` file.

**Managing via git-submodules**:

For adding a new sub-module,

```
$ git submodule add https://github.com/prataprc/tree-sitter-toml src/tss/toml
$ git commit -am 'ts/toml: add new tree-sitter parser'
$ git push origin master
```

Cloning `ted` with sub-modules,

```
$ git clone https://github.com/prataprc/ted
$ git submodule init
$ git submodule update
```

To remove a sub-module

```
$ git submodule deinit -f toml
$ rm -rf ted/.git/modules/ts
$ git rm -f toml
```

[tree-sitter]: https://tree-sitter.github.io
