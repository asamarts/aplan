# Reproduce from the repository root after `mbx build -p aplan`.
Require git
Require sed

Output docs/demos/tui-work-loop.gif

Set Shell bash
Set Columns 120
Set Rows 34
Set FontSize 16
Set TypingSpeed 15ms
Set PlaybackSpeed 1.5
Set CursorBlink false
Set Theme "Catppuccin Frappe"

# Copy the tracked safe fixture to disk-backed scratch and give evidence a
# real repository identity. Setup and cleanup are intentionally not recorded.
Hide
Type `APLAN_DEMO_BIN="$PWD/target/debug/aplan"; DEMO_DIR="$(mktemp -d "$HOME/.tmp/aplan-tui-demo.XXXXXX")"; cp -R docs/fixtures/tui-work-loop/. "$DEMO_DIR/"; cd "$DEMO_DIR"; git init -q; git config user.name "Demo User"; git config user.email demo@example.invalid; git add .; git commit -qm "Seed safe TUI demo fixture"; clear`
Enter
Show

# The initial frame names real loading phases before plan I/O finishes.
Type `"$APLAN_DEMO_BIN" tui --ascii --reduced-motion --theme dark`
Enter
Wait+Screen@30s /Overview - where attention goes next/
Sleep 2s

# CHECK TO FIX: inspect APLAN009, leave the full-screen session safely, repair
# the source file, then relaunch and observe a clean structural verdict.
Type "3"
Wait+Screen@10s /APLAN009/
Sleep 2s
Type "q"
Wait+Screen@10s /\$/
Type `sed -i '/status: todo/a effort_days: 1' plan/tasks/T-001.md`
Enter
Type `printf '\n// Demonstrated work.\n' >> src/lib.rs; git add plan/tasks/T-001.md src/lib.rs; git commit -qm "Add the required effort"`
Enter
Type `"$APLAN_DEMO_BIN" tui --ascii --reduced-motion --theme dark`
Enter
Wait+Screen@30s /Overview - where attention goes next/
Type "3"
Wait+Screen@10s /check PASS E0 W0/
Sleep 2s

# READY TO CLOSE: locate the task, discover Close through the action palette,
# review the exact candidate change, confirm, and observe the committed verdict.
Type "2"
Wait+Screen@10s /Repair and close the demo task/
Ctrl+P
Wait+Screen@10s /Action:/
Type "Close task with evidence"
Wait+Screen@10s /Close task with evidence/
Enter
Wait+Screen@20s /FORM: Close task with evidence/
Shift+Tab
Enter
Wait+Screen@20s /PREVIEW: Close task with evidence/
Shift+Tab
Enter
Wait+Screen@20s /Verdict: COMMITTED/
Sleep 3s
Escape
Wait+Screen@10s /T-001.*done/
Sleep 3s
Type "q"
Sleep 1s

Hide
Type `cd /; case "$DEMO_DIR" in "$HOME"/.tmp/aplan-tui-demo.*) rm -rf -- "$DEMO_DIR" ;; *) exit 1 ;; esac`
Enter
