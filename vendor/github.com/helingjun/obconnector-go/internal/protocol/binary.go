package protocol

import (
	"encoding/binary"
	"fmt"
	"io"
	"math"
	"time"
)

func AppendBinaryParam(buf []byte, typ byte, value any) ([]byte, error) {
	switch v := value.(type) {
	case nil:
		return buf, nil
	case int64:
		return binary.LittleEndian.AppendUint64(buf, uint64(v)), nil
	case float64:
		return binary.LittleEndian.AppendUint64(buf, math.Float64bits(v)), nil
	case string:
		return PutLengthEncodedString(buf, v), nil
	case []byte:
		return PutLengthEncodedString(buf, string(v)), nil
	case time.Time:
		return AppendBinaryTime(buf, v), nil
	case bool:
		if v {
			return append(buf, 1), nil
		}
		return append(buf, 0), nil
	default:
		return nil, fmt.Errorf("oceanbase: binary parameter type %T not supported", value)
	}
}

func AppendBinaryTime(buf []byte, t time.Time) []byte {
	if t.IsZero() {
		return append(buf, 0)
	}
	year, month, day := t.Date()
	hour, min, sec := t.Clock()
	nsec := t.Nanosecond()

	if nsec != 0 {
		buf = append(buf, 11)
		buf = binary.LittleEndian.AppendUint16(buf, uint16(year))
		buf = append(buf, byte(month), byte(day), byte(hour), byte(min), byte(sec))
		buf = binary.LittleEndian.AppendUint32(buf, uint32(nsec/1000))
	} else if hour != 0 || min != 0 || sec != 0 {
		buf = append(buf, 7)
		buf = binary.LittleEndian.AppendUint16(buf, uint16(year))
		buf = append(buf, byte(month), byte(day), byte(hour), byte(min), byte(sec))
	} else {
		buf = append(buf, 4)
		buf = binary.LittleEndian.AppendUint16(buf, uint16(year))
		buf = append(buf, byte(month), byte(day))
	}
	return buf
}

func ParseBinaryRow(packet []byte, columnCount int, types []byte) ([]any, error) {
	if len(packet) == 0 || packet[0] != 0x00 {
		return nil, fmt.Errorf("invalid binary row header")
	}
	pos := 1
	nullBitmapLen := (columnCount + 7 + 2) / 8
	if len(packet) < pos+nullBitmapLen {
		return nil, io.ErrUnexpectedEOF
	}
	nullBitmap := packet[pos : pos+nullBitmapLen]
	pos += nullBitmapLen

	row := make([]any, columnCount)
	for i := 0; i < columnCount; i++ {
		if (nullBitmap[(i+2)/8] & (1 << ((i + 2) % 8))) != 0 {
			row[i] = nil
			continue
		}
		val, used, err := ParseBinaryValue(packet[pos:], types[i])
		if err != nil {
			return nil, err
		}
		row[i] = val
		pos += used
	}
	return row, nil
}

func ParseBinaryValue(data []byte, typ byte) (any, int, error) {
	switch typ {
	case ColumnTypeTiny:
		if len(data) < 1 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return int64(int8(data[0])), 1, nil
	case ColumnTypeShort, ColumnTypeYear:
		if len(data) < 2 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return int64(int16(binary.LittleEndian.Uint16(data[:2]))), 2, nil
	case ColumnTypeLong, ColumnTypeInt24:
		if len(data) < 4 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return int64(int32(binary.LittleEndian.Uint32(data[:4]))), 4, nil
	case ColumnTypeLongLong:
		if len(data) < 8 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return int64(binary.LittleEndian.Uint64(data[:8])), 8, nil
	case ColumnTypeFloat:
		if len(data) < 4 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return float64(math.Float32frombits(binary.LittleEndian.Uint32(data[:4]))), 4, nil
	case ColumnTypeDouble:
		if len(data) < 8 {
			return nil, 0, io.ErrUnexpectedEOF
		}
		return math.Float64frombits(binary.LittleEndian.Uint64(data[:8])), 8, nil
	case ColumnTypeDate, ColumnTypeTimestamp, ColumnTypeDateTime:
		return ParseBinaryTime(data)
	case ColumnTypeDecimal, ColumnTypeNewDecimal, ColumnTypeVarChar, ColumnTypeVarString, ColumnTypeString, ColumnTypeBlob, ColumnTypeTinyBlob, ColumnTypeMediumBlob, ColumnTypeLongBlob,
		ColumnTypeOracleTimestampTZ, ColumnTypeOracleTimestampLTZ, ColumnTypeOracleTimestampNano,
		ColumnTypeOracleRaw, ColumnTypeOracleIntervalYM, ColumnTypeOracleIntervalDS,
		ColumnTypeOracleNumberFloat, ColumnTypeOracleNVarChar2, ColumnTypeOracleNChar,
		ColumnTypeOracleRowID, ColumnTypeOracleBlob, ColumnTypeOracleClob:
		s, used, _, err := ReadLengthEncodedString(data)
		if err != nil {
			return nil, 0, err
		}
		return string(s), used, nil
	default:
		return nil, 0, fmt.Errorf("oceanbase: unsupported binary column type %02x", typ)
	}
}

func ParseBinaryTime(data []byte) (any, int, error) {
	if len(data) == 0 {
		return nil, 0, io.ErrUnexpectedEOF
	}
	n := int(data[0])
	if n == 0 {
		return time.Time{}, 1, nil
	}
	if len(data) < n+1 {
		return nil, 0, io.ErrUnexpectedEOF
	}
	year := int(binary.LittleEndian.Uint16(data[1:3]))
	month := time.Month(data[3])
	day := int(data[4])
	var hour, min, sec, nsec int
	if n >= 7 {
		hour = int(data[5])
		min = int(data[6])
		sec = int(data[7])
	}
	if n >= 11 {
		nsec = int(binary.LittleEndian.Uint32(data[8:12])) * 1000
	}
	return time.Date(year, month, day, hour, min, sec, nsec, time.UTC), n + 1, nil
}

func GetBinaryParamType(value any) byte {
	switch value.(type) {
	case nil:
		return ColumnTypeNull
	case int64:
		return ColumnTypeLongLong
	case float64:
		return ColumnTypeDouble
	case string, []byte:
		return ColumnTypeVarString
	case time.Time:
		return ColumnTypeDateTime
	case bool:
		return ColumnTypeTiny
	default:
		return ColumnTypeVarString
	}
}
