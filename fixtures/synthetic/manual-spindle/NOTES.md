# manual-spindle

No `M3`, because the spindle is started by hand. With `spindle_manual = true`,
`modal.spindle_stopped` and `modal.spindle_speed_zero` must be skipped with a
reason. The same file with `spindle_manual = false` must raise them.
