package main

import (
	"fmt"
	"os"

	"onetcli-db-ipc-drivers/internal/drivers/oceanbase"
	"onetcli-db-ipc-drivers/internal/runner"
)

func main() {
	if err := runner.Run(oceanbase.Spec()); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
