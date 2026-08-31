package main

import (
	"fmt"
	"os"

	"navop-db-ipc-drivers/internal/drivers/gbase8s"
	"navop-db-ipc-drivers/internal/runner"
)

func main() {
	if err := runner.Run(gbase8s.Spec()); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
