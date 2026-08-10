//go:build oceanbase_driver

package main

import (
	"database/sql"
	"testing"
)

func TestOceanBaseBinaryRegistersRequiredSQLDrivers(t *testing.T) {
	registered := map[string]bool{}
	for _, name := range sql.Drivers() {
		registered[name] = true
	}

	for _, name := range []string{"mysql", "oboracle", "oracle"} {
		if !registered[name] {
			t.Fatalf("SQL driver %q is not registered; registered drivers: %v", name, sql.Drivers())
		}
	}
}
