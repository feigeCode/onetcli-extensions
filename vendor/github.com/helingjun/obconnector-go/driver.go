package oceanbase

import (
	"context"
	"database/sql"
	"database/sql/driver"
)

func init() {
	sql.Register("oceanbase", &Driver{})
	sql.Register("oboracle", &Driver{})
}

type Driver struct{}

func (d *Driver) Open(name string) (driver.Conn, error) {
	cfg, err := ParseDSN(name)
	if err != nil {
		return nil, err
	}
	return (&Connector{cfg: cfg}).Connect(context.Background())
}

func (d *Driver) OpenConnector(name string) (driver.Connector, error) {
	cfg, err := ParseDSN(name)
	if err != nil {
		return nil, err
	}
	return &Connector{cfg: cfg}, nil
}

type Connector struct {
	cfg *Config
}

func NewConnector(cfg Config) (driver.Connector, error) {
	if err := cfg.normalize(); err != nil {
		return nil, err
	}
	return &Connector{cfg: &cfg}, nil
}

func (c *Connector) Connect(ctx context.Context) (driver.Conn, error) {
	if err := c.cfg.normalize(); err != nil {
		return nil, err
	}
	conn, err := dialAndHandshake(ctx, c.cfg)
	if err != nil {
		return nil, err
	}
	return conn, nil
}

func (c *Connector) Driver() driver.Driver {
	return &Driver{}
}
