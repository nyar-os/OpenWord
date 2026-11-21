# @wae/open-word-skills

Agent skills for OpenWord. The package ships skill documents only; it is not a
runtime library and contains no XML, ZIP, style, template or conversion
algorithms.

## Status

Scaffold only. `skills/open-word/SKILL.md` records the routing and evidence rules
for create, edit, template fill, review and delivery, but no SDK or CLI capability
exists yet, so none of those routes is executable.

## Layout

```text
packages/open-word-skills/
├── package.json
├── README.md
└── skills/
    └── open-word/
        └── SKILL.md
```

Design rules, the format/support matrix and executable examples are added with
the first implemented release. The skill must never be shipped claiming methods,
commands or flags that the installed SDK/CLI does not expose.
