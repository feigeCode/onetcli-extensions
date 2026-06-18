package dbipc

const (
	ErrParseError       int32 = -32700
	ErrInvalidRequest   int32 = -32600
	ErrMethodNotFound   int32 = -32601
	ErrInvalidParams    int32 = -32602
	ErrInternalError    int32 = -32603
	ErrNotInitialized   int32 = -32001
	ErrUnknownConnID    int32 = -32007
	ErrUnknownCursorID  int32 = -32008
	ErrConnectionFailed int32 = -33001
	ErrSQLSyntax        int32 = -34001
	ErrNotSupported     int32 = -35001
)
