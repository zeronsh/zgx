-- Stamps: a mark before the title, a friendlier empty state in place of
-- the app's, and each row wrapped with a "done" badge by replacing the
-- app's row module (the replacement runs in the app's VM, in its place).
ui.contribute { target = "#title", before = ui.text { id = "stamp", "✓" } }
ui.contribute { target = "#empty", replace = ui.text { id = "all-clear", "All clear: add a task above." } }
ui.override { module = "@app/components/task_row", with = "./task_row" }
