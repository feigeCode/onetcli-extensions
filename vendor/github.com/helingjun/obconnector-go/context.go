package oceanbase

import (
	"context"
)

type contextKey int

const (
	partitionIDKey contextKey = iota
	traceIDKey
	spanIDKey
)

func WithPartitionID(ctx context.Context, id int64) context.Context {
	return context.WithValue(ctx, partitionIDKey, id)
}

func WithTraceID(ctx context.Context, id string) context.Context {
	return context.WithValue(ctx, traceIDKey, id)
}

func WithSpanID(ctx context.Context, id string) context.Context {
	return context.WithValue(ctx, spanIDKey, id)
}

func partitionIDFromContext(ctx context.Context) (int64, bool) {
	id, ok := ctx.Value(partitionIDKey).(int64)
	return id, ok
}

func traceIDFromContext(ctx context.Context) (string, bool) {
	id, ok := ctx.Value(traceIDKey).(string)
	return id, ok
}

func spanIDFromContext(ctx context.Context) (string, bool) {
	id, ok := ctx.Value(spanIDKey).(string)
	return id, ok
}
