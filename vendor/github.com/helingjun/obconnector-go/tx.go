package oceanbase

import "context"

type Tx struct {
	conn *Conn
	done bool
}

func (tx *Tx) Commit() error {
	return tx.finish(context.Background(), "commit")
}

func (tx *Tx) Rollback() error {
	return tx.finish(context.Background(), "rollback")
}

func (tx *Tx) finish(ctx context.Context, query string) error {
	if tx.done {
		return nil
	}
	tx.conn.mu.Lock()
	defer tx.conn.mu.Unlock()
	if err := tx.conn.checkUsableLocked(); err != nil {
		return err
	}
	_, err := tx.conn.execLocked(ctx, query)
	if err == nil {
		tx.done = true
		tx.conn.inTx = false
	}
	return tx.conn.markBadIfConnErr(err)
}
