package oceanbase

import (
	"context"
	"database/sql/driver"
	"errors"
)

type Stmt struct {
	conn        *Conn
	query       string
	stmtID      uint32
	paramCount  int
	columnCount int
	closed      bool
}

func (s *Stmt) Close() error {
	if s.closed {
		return nil
	}
	s.closed = true
	if s.conn == nil {
		return nil
	}
	return s.conn.closeStmt(s.stmtID)
}

func (s *Stmt) NumInput() int {
	return s.paramCount
}

func (s *Stmt) Exec(args []driver.Value) (driver.Result, error) {
	return s.ExecContext(context.Background(), valuesToNamed(args))
}

func (s *Stmt) Query(args []driver.Value) (driver.Rows, error) {
	return s.QueryContext(context.Background(), valuesToNamed(args))
}

func (s *Stmt) ExecContext(ctx context.Context, args []driver.NamedValue) (driver.Result, error) {
	if s.closed {
		return nil, errors.New("oceanbase: statement is closed")
	}
	s.conn.mu.Lock()
	defer s.conn.mu.Unlock()
	if err := s.conn.checkUsableLocked(); err != nil {
		return nil, err
	}
	res, err := s.conn.stmtExecLocked(ctx, s.stmtID, args)
	if err != nil {
		return nil, s.conn.markBadIfConnErr(err)
	}
	return res, nil
}

func (s *Stmt) QueryContext(ctx context.Context, args []driver.NamedValue) (driver.Rows, error) {
	if s.closed {
		return nil, errors.New("oceanbase: statement is closed")
	}
	s.conn.mu.Lock()
	if err := s.conn.checkUsableLocked(); err != nil {
		s.conn.mu.Unlock()
		return nil, err
	}
	rows, err := s.conn.stmtQueryLocked(ctx, s.stmtID, args)
	if err != nil {
		s.conn.mu.Unlock()
		return nil, s.conn.markBadIfConnErr(err)
	}
	if r, ok := rows.(*Rows); ok && r.streaming {
		r.release = s.conn.mu.Unlock
	} else {
		s.conn.mu.Unlock()
	}
	return rows, nil
}

func (s *Stmt) BulkExecContext(ctx context.Context, argRows [][]driver.NamedValue) (driver.Result, error) {
	if s.closed {
		return nil, errors.New("oceanbase: statement is closed")
	}
	s.conn.mu.Lock()
	defer s.conn.mu.Unlock()
	if err := s.conn.checkUsableLocked(); err != nil {
		return nil, err
	}
	res, err := s.conn.stmtBulkExecLocked(ctx, s.stmtID, argRows)
	if err != nil {
		return nil, s.conn.markBadIfConnErr(err)
	}
	return res, nil
}

func (s *Stmt) Reset(ctx context.Context) error {
	if s.closed {
		return errors.New("oceanbase: statement is closed")
	}
	return s.conn.resetStmt(s.stmtID)
}
