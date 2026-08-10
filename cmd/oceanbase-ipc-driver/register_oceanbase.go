//go:build oceanbase_driver

package main

import (
	_ "github.com/go-sql-driver/mysql"
	_ "github.com/helingjun/obconnector-go"
	_ "github.com/sijms/go-ora/v2"
)
