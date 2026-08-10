package oceanbase

import (
	"database/sql/driver"
	"encoding/hex"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"
)

func interpolateParams(query string, args []driver.NamedValue) (string, error) {
	if len(args) == 0 {
		if countPlaceholders(query) > 0 {
			return "", errors.New("oceanbase: not enough query arguments")
		}
		return query, nil
	}

	var out strings.Builder
	out.Grow(len(query) + len(args)*8)
	argPos := 0
	state := sqlNormal

	for i := 0; i < len(query); i++ {
		ch := query[i]
		switch state {
		case sqlNormal:
			switch ch {
			case '\'':
				state = sqlSingleQuote
				out.WriteByte(ch)
			case '"':
				state = sqlDoubleQuote
				out.WriteByte(ch)
			case '`':
				state = sqlBacktick
				out.WriteByte(ch)
			case '#':
				state = sqlLineComment
				out.WriteByte(ch)
			case '-':
				if i+1 < len(query) && query[i+1] == '-' {
					state = sqlLineComment
					out.WriteString("--")
					i++
				} else {
					out.WriteByte(ch)
				}
			case '/':
				if i+1 < len(query) && query[i+1] == '*' {
					state = sqlBlockComment
					out.WriteString("/*")
					i++
				} else {
					out.WriteByte(ch)
				}
			case '?':
				if argPos >= len(args) {
					return "", errors.New("oceanbase: not enough query arguments")
				}
				lit, err := literal(args[argPos])
				if err != nil {
					return "", err
				}
				out.WriteString(lit)
				argPos++
			default:
				out.WriteByte(ch)
			}
		case sqlSingleQuote:
			out.WriteByte(ch)
			if ch == '\'' {
				if i+1 < len(query) && query[i+1] == '\'' {
					out.WriteByte(query[i+1])
					i++
				} else {
					state = sqlNormal
				}
			}
		case sqlDoubleQuote:
			out.WriteByte(ch)
			if ch == '"' {
				state = sqlNormal
			}
		case sqlBacktick:
			out.WriteByte(ch)
			if ch == '`' {
				state = sqlNormal
			}
		case sqlLineComment:
			out.WriteByte(ch)
			if ch == '\n' {
				state = sqlNormal
			}
		case sqlBlockComment:
			out.WriteByte(ch)
			if ch == '*' && i+1 < len(query) && query[i+1] == '/' {
				out.WriteByte('/')
				i++
				state = sqlNormal
			}
		}
	}
	if argPos != len(args) {
		return "", errors.New("oceanbase: too many query arguments")
	}
	return out.String(), nil
}

func countPlaceholders(query string) int {
	count := 0
	state := sqlNormal
	for i := 0; i < len(query); i++ {
		ch := query[i]
		switch state {
		case sqlNormal:
			switch ch {
			case '\'':
				state = sqlSingleQuote
			case '"':
				state = sqlDoubleQuote
			case '`':
				state = sqlBacktick
			case '#':
				state = sqlLineComment
			case '-':
				if i+1 < len(query) && query[i+1] == '-' {
					i++
					state = sqlLineComment
				}
			case '/':
				if i+1 < len(query) && query[i+1] == '*' {
					i++
					state = sqlBlockComment
				}
			case '?':
				count++
			}
		case sqlSingleQuote:
			if ch == '\'' {
				if i+1 < len(query) && query[i+1] == '\'' {
					i++
				} else {
					state = sqlNormal
				}
			}
		case sqlDoubleQuote:
			if ch == '"' {
				state = sqlNormal
			}
		case sqlBacktick:
			if ch == '`' {
				state = sqlNormal
			}
		case sqlLineComment:
			if ch == '\n' {
				state = sqlNormal
			}
		case sqlBlockComment:
			if ch == '*' && i+1 < len(query) && query[i+1] == '/' {
				i++
				state = sqlNormal
			}
		}
	}
	return count
}

type sqlState int

const (
	sqlNormal sqlState = iota
	sqlSingleQuote
	sqlDoubleQuote
	sqlBacktick
	sqlLineComment
	sqlBlockComment
)

func literal(arg driver.NamedValue) (string, error) {
	if arg.Name != "" {
		return "", errors.New("oceanbase: named parameters are not implemented")
	}
	switch v := arg.Value.(type) {
	case nil:
		return "NULL", nil
	case int64:
		return strconv.FormatInt(v, 10), nil
	case float64:
		return strconv.FormatFloat(v, 'g', -1, 64), nil
	case bool:
		if v {
			return "1", nil
		}
		return "0", nil
	case []byte:
		return "hextoraw('" + strings.ToUpper(hex.EncodeToString(v)) + "')", nil
	case string:
		return quoteString(v), nil
	case time.Time:
		return "timestamp " + quoteString(v.Format("2006-01-02 15:04:05.999999999")), nil
	default:
		return "", fmt.Errorf("oceanbase: unsupported parameter type %T", arg.Value)
	}
}

func quoteString(s string) string {
	return "'" + strings.ReplaceAll(s, "'", "''") + "'"
}

func valuesToNamed(values []driver.Value) []driver.NamedValue {
	if len(values) == 0 {
		return nil
	}
	named := make([]driver.NamedValue, len(values))
	for i, value := range values {
		named[i] = driver.NamedValue{Ordinal: i + 1, Value: value}
	}
	return named
}
