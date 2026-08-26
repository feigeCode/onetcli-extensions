package gbase8s

// buildPumpArgs assembles mysqlpump arguments that match the official GBase 8s
// dump utility so exported structures carry comments and use the expected
// time literals.
func buildPumpArgs(host, port, user, password, database, schema string) []string {
	args := []string{
		"--set-charset",
		"--routines",
		"--events",
		"--databases", database,
	}
	// Preserve table/column comments and avoid rewriting timestamps into
	// numeric literals, which the official tool keeps as quoted strings.
	args = append(args,
		"--skip-optimize-timezone",
		"--comments",
		"--add-drop-table",
	)
	if schema != "" {
		args = append(args, "--tables", schema)
	}
	return args
}
