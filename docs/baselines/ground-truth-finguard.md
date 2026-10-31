# Ground Truth — finguard TS/TSX, 20 call sites (FILLED 2026-10-31)

- Filled: 2026-10-31, agent verification (owner delegated), working tree at finguard HEAD 2026-10-31.
- Verification legend: `[x]` = verified by direct code read (import line + call line).
- Known duplicate-name trap: `normalizeVndInput` is defined TWICE — apps/prototype/src/lib/format.ts:13 and packages/core-ledger/src/parser.ts:110 (module-local). Per-module import maps decide the target (G01 vs G02); kg must not merge them.
- External imports (react; @finguard/* workspace aliases) bind but resolve to
  unindexed modules by design (pnpm workspace alias resolution is out of P2
  scope) — calls into them drop with the builtin counter. NOTE (R2): these are
  real intra-monorepo value calls (e.g. new DomainError in command-bus.ts:91);
  they are excluded from GT because the alias targets are not indexed, not
  because the calls are type-only. Module stems keep their dots (runner.node).

| # | caller file:line | callee qualname | invocation kind | verified |
|---|------------------|-----------------|-----------------|----------|
| G01 | apps/prototype/src/lib/demo-parser.ts:10 | apps.prototype.src.lib.format.normalizeVndInput | import-call (rel import demo-parser.ts:2 './format') | [x] |
| G02 | packages/core-ledger/src/parser.ts:107 | packages.core-ledger.src.parser.normalizeVndInput | same-file local def (parser.ts:110) — must NOT resolve to the prototype copy | [x] |
| G03 | apps/prototype/src/features/overview/OverviewPage.tsx:26 | apps.prototype.src.lib.format.formatMoney | import-call (OverviewPage.tsx:2 '../../lib/format') | [x] |
| G04 | apps/prototype/src/features/overview/OverviewPage.tsx:42 | apps.prototype.src.lib.format.formatMoney | import-call | [x] |
| G05 | apps/prototype/src/features/chat/ProposalCard.tsx:33 | apps.prototype.src.lib.format.formatMoney | import-call | [x] |
| G06 | apps/prototype/src/features/chat/ChatPage.tsx:21 | apps.prototype.src.lib.demo-parser.parseDemoMessage | import-call (ChatPage.tsx:3) | [x] |
| G07 | packages/db/src/__tests__/command-bus.test.ts:63 | packages.db.src.command-bus.CommandBus.constructor | import-call constructor (import :6 '../command-bus'; new Expression; ctor at command-bus.ts:33) | [x] |
| G08 | packages/db/src/__tests__/command-bus.test.ts:76 | packages.db.src.command-bus.CommandBus.constructor | import-call constructor | [x] |
| G09 | packages/db/src/__tests__/command-bus.test.ts:93 | packages.db.src.command-bus.CommandBus.constructor | import-call constructor | [x] |
| G10 | packages/db/src/__tests__/migration.test.ts:7 | packages.db.src.connection.initializeDatabase | import-call (migration.test.ts:2 '../connection') | [x] |
| G11 | packages/db/src/__tests__/command-bus.test.ts:9 | packages.db.src.connection.initializeDatabase | import-call (command-bus.test.ts:8) | [x] |
| G12 | packages/db/src/__tests__/migration.test.ts:8 | packages.db.src.migrations.runner.node.runMigrations | import-call (migration.test.ts:3 '../migrations/runner.node') | [x] |
| G13 | packages/db/src/__tests__/migration.test.ts:31 | packages.db.src.migrations.runner.node.runMigrations | import-call | [x] |
| G14 | apps/prototype/src/app/AppLayout.tsx:18 | apps.prototype.src.lib.i18n.t | import-call (AppLayout.tsx:2 '../lib/i18n') | [x] |
| G15 | apps/prototype/src/app/AppLayout.tsx:19 | apps.prototype.src.lib.i18n.t | import-call | [x] |
| G16 | packages/db/src/migrations/runner.browser.ts:21 | packages.db.src.migrations.core.applyMigrations | import-call (runner.browser.ts:2 './core') | [x] |
| G17 | apps/prototype/src/features/chat/ProposalCard.tsx:30 | apps.prototype.src.lib.i18n.t | import-call (ProposalCard.tsx:4, relative ../../lib/i18n) | [x] |
| G18 | apps/prototype/src/features/chat/ProposalCard.tsx:39 | apps.prototype.src.lib.i18n.t | import-call | [x] |
| G19 | packages/db/src/__tests__/command-bus.test.ts:185 | packages.db.src.command-bus.loadTransaction | import-call (function, :6) | [x] |
| G20 | apps/prototype/src/app/App.tsx:17 | apps.prototype.src.app.providers.use-demo-ledger.useDemoLedger | import-call hook (App.tsx:3 './providers/use-demo-ledger') | [x] |

Notes for G2 scoring:
- Module paths use kg's convention: path segments dot-joined, extension stripped, dashes preserved inside segments (core-ledger, use-demo-ledger, runner-node).
- G02 is the hard trap: a merged or cross-package resolution is a precision MISS.
- G07-G09 exercise the constructor redirect (new Expression -> class -> constructor method).
