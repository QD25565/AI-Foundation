@echo off
:: AI-Foundation Integration + Regression Test Runner
:: Runs the full test suite: 17 suites, 152 tests (151 pass, 2 ignored)
::
:: Usage:
::   run-all-tests.bat              -- run all tests
::   run-all-tests.bat --update-golden  -- regenerate golden baseline files
::
:: Test suites:
::   batches            (8)   task batches lifecycle
::   broadcast          (5)   team broadcasts
::   dialogues          (10)  n-party round-robin dialogues
::   federation_null    (19)  federation identity + consent (no network)
::   file_claims        (5)   file ownership claims
::   golden_outputs     (6)   snapshot tests — detect truncation/format regressions
::   learnings          (9)   team playbook learnings
::   messaging          (5)   direct messages
::   mcp_conformance    (8)   MCP server JSON-RPC end-to-end (stdio transport)
::   presence_and_actions(12) presence updates, file actions, DM read acks
::   projects           (17)  projects + features CRUD (1 ignored: name update)
::   regression         (11)  named regressions REG-001..011 (REG-006 ignored)
::   rooms              (13)  persistent collaborative rooms
::   tasks              (9)   task lifecycle + state machine
::   trust              (9)   trust scores + feedback
::   votes              (7)   voting lifecycle
::
:: Golden tests require baseline files in tests/golden/.
:: Regenerate baselines after intentional format changes:
::   run-all-tests.bat --update-golden

setlocal

cd /d "%~dp0"

if "%1"=="--update-golden" (
    echo [GOLDEN] Regenerating golden baseline files...
    set UPDATE_GOLDEN=1
    cargo test --test golden_outputs -- --test-threads=1
    if errorlevel 1 (
        echo [FAIL] Golden regeneration failed.
        exit /b 1
    )
    echo [OK] Golden files updated. Review tests\golden\ before committing.
    goto :eof
)

:: Run full suite with parallelism — each test gets isolated TempDir + unique pipe
cargo test -- --test-threads=4

if errorlevel 1 (
    echo.
    echo [FAIL] Some tests failed — see output above.
    exit /b 1
) else (
    echo.
    echo [OK] All tests passed.
    exit /b 0
)
