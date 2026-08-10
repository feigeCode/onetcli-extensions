package oceanbase

import (
	"database/sql/driver"
	"encoding/binary"
	"fmt"
	"io"
	"reflect"
	"strconv"
	"time"

	"github.com/helingjun/obconnector-go/internal/protocol"
)

type columnDef struct {
	name         string
	typ          byte
	flags        uint16
	decimals     byte
	columnLength uint32
	charset      uint16
}

const notNullFlag uint16 = 0x0001

type Rows struct {
	conn      *Conn
	colDefs   []columnDef
	columns   []string
	types     []byte
	values    [][]driver.Value
	pos       int
	streaming bool
	binary    bool
	done      bool
	closed    bool
	release   func()
}

func (r *Rows) Columns() []string {
	return append([]string(nil), r.columns...)
}

func (r *Rows) Close() error {
	if r.closed {
		return nil
	}
	r.closed = true
	r.values = nil
	var err error
	if r.streaming && !r.done {
		err = r.drain()
	}
	r.finish()
	return err
}

func (r *Rows) finish() {
	r.done = true
	if r.release != nil {
		r.release()
		r.release = nil
	}
}

func (r *Rows) drain() error {
	const maxDrainRows = 10000
	for i := 0; i < maxDrainRows; i++ {
		packet, err := r.conn.packets.ReadPacket()
		if err != nil {
			_ = r.conn.markBadIfConnErr(err)
			return err
		}
		if isEOFOrOK(packet) {
			return nil
		}
		if len(packet) > 0 && packet[0] == protocol.ErrPacket {
			return parseServerError(packet)
		}
	}
	r.conn.bad.Store(true)
	return fmt.Errorf("oceanbase: drain() exceeded %d rows, possible protocol desync", maxDrainRows)
}

func (r *Rows) nextStreaming(dest []driver.Value) error {
	if r.done {
		return io.EOF
	}
	packet, err := r.conn.packets.ReadPacket()
	if err != nil {
		_ = r.conn.markBadIfConnErr(err)
		r.finish()
		return err
	}
	if isEOFOrOK(packet) {
		r.finish()
		return io.EOF
	}
	if len(packet) > 0 && packet[0] == protocol.ErrPacket {
		r.finish()
		return parseServerError(packet)
	}
	var row []driver.Value
	if r.binary {
		binaryRow, err := protocol.ParseBinaryRow(packet, len(r.columns), r.types)
		if err != nil {
			r.finish()
			return err
		}
		row = make([]driver.Value, len(binaryRow))
		for i, v := range binaryRow {
			row[i] = v
		}
	} else {
		textRow, err := parseTextRow(packet, len(r.columns), r.types)
		if err != nil {
			r.finish()
			return err
		}
		row = textRow
	}
	copy(dest, row)
	return nil
}

func (r *Rows) Next(dest []driver.Value) error {
	if r.streaming {
		return r.nextStreaming(dest)
	}
	if r.pos >= len(r.values) {
		return io.EOF
	}
	copy(dest, r.values[r.pos])
	r.pos++
	return nil
}

func (r *Rows) ColumnTypeDatabaseTypeName(index int) string {
	if index < 0 || index >= len(r.types) {
		return ""
	}
	return databaseTypeName(r.types[index])
}

func (r *Rows) ColumnTypeScanType(index int) reflect.Type {
	if index < 0 || index >= len(r.types) {
		return reflect.TypeOf("")
	}
	return scanType(r.types[index])
}

func (r *Rows) ColumnTypeNullable(index int) (nullable, ok bool) {
	if index < 0 || index >= len(r.colDefs) {
		return true, true
	}
	return r.colDefs[index].flags&notNullFlag == 0, true
}

func (r *Rows) ColumnTypeLength(index int) (length int64, ok bool) {
	if index < 0 || index >= len(r.colDefs) {
		return 0, false
	}
	return int64(r.colDefs[index].columnLength), true
}

func (r *Rows) ColumnTypePrecisionScale(index int) (precision, scale int64, ok bool) {
	if index < 0 || index >= len(r.colDefs) {
		return 0, 0, false
	}
	cd := r.colDefs[index]
	precision = int64(cd.columnLength)
	scale = int64(cd.decimals)
	return precision, scale, true
}

func (c *Conn) readQueryResult() (driver.Rows, error) {
	first, err := c.packets.ReadPacket()
	if err != nil {
		return nil, err
	}
	if len(first) == 0 {
		return nil, io.ErrUnexpectedEOF
	}
	if first[0] == protocol.ErrPacket {
		return nil, parseServerError(first)
	}
	if first[0] == protocol.OKPacket {
		return &Rows{done: true}, nil
	}

	columnCount, _, _, err := protocol.ReadLengthEncodedInt(first)
	if err != nil {
		return nil, err
	}
	colDefs := make([]columnDef, 0, columnCount)
	columns := make([]string, 0, columnCount)
	types := make([]byte, 0, columnCount)
	for i := uint64(0); i < columnCount; i++ {
		packet, err := c.packets.ReadPacket()
		if err != nil {
			return nil, err
		}
		cd, err := parseColumnDefinition(packet)
		if err != nil {
			return nil, err
		}
		colDefs = append(colDefs, cd)
		columns = append(columns, cd.name)
		types = append(types, cd.typ)
	}
	if err := c.readEOFOrOK(); err != nil {
		return nil, err
	}

	return &Rows{conn: c, colDefs: colDefs, columns: columns, types: types, streaming: true}, nil
}

func (c *Conn) readResultFromFirstPacket(packet []byte) (driver.Result, error) {
	if len(packet) == 0 {
		return nil, io.ErrUnexpectedEOF
	}
	switch packet[0] {
	case protocol.OKPacket:
		res, status, err := c.handleOK(packet)
		if err != nil {
			return nil, err
		}
		if status&protocol.ServerMoreResultsExists != 0 {
			if err := c.drainRemainingResults(); err != nil {
				return nil, err
			}
		}
		return res, nil
	case protocol.ErrPacket:
		return nil, parseServerError(packet)
	default:
		if _, err := c.readQueryResultAfterColumnCount(packet); err != nil {
			return nil, err
		}
		return result{}, nil
	}
}

func (c *Conn) readQueryResultAfterColumnCount(first []byte) (driver.Rows, error) {
	columnCount, _, _, err := protocol.ReadLengthEncodedInt(first)
	if err != nil {
		return nil, err
	}
	for i := uint64(0); i < columnCount; i++ {
		if _, err := c.packets.ReadPacket(); err != nil {
			return nil, err
		}
	}
	if err := c.readEOFOrOK(); err != nil {
		return nil, err
	}
	const maxDrainRows = 10000
	for i := 0; i < maxDrainRows; i++ {
		packet, err := c.packets.ReadPacket()
		if err != nil {
			return nil, err
		}
		if isEOFOrOK(packet) {
			return &Rows{}, nil
		}
	}
	c.bad.Store(true)
	return nil, fmt.Errorf("oceanbase: readQueryResultAfterColumnCount exceeded %d rows, possible protocol desync", maxDrainRows)
}

func (c *Conn) readEOFOrOK() error {
	packet, err := c.packets.ReadPacket()
	if err != nil {
		return err
	}
	if len(packet) > 0 && packet[0] == protocol.ErrPacket {
		return parseServerError(packet)
	}
	if !isEOFOrOK(packet) {
		return fmt.Errorf("expected EOF/OK packet, got 0x%02x", packet[0])
	}
	return nil
}

func isEOFOrOK(packet []byte) bool {
	if len(packet) == 0 {
		return false
	}
	switch packet[0] {
	case protocol.ErrPacket:
		return false
	case protocol.OKPacket:
		_, used, _, err := protocol.ReadLengthEncodedInt(packet[1:])
		if err != nil {
			return false
		}
		_, _, _, err = protocol.ReadLengthEncodedInt(packet[1+used:])
		return err == nil
	case protocol.EOFPacket:
		return true
	}
	return false
}

func parseColumnDefinition(packet []byte) (columnDef, error) {
	pos := 0
	for i := 0; i < 4; i++ {
		_, used, _, err := protocol.ReadLengthEncodedString(packet[pos:])
		if err != nil {
			return columnDef{}, err
		}
		pos += used
	}
	nameBytes, used, _, err := protocol.ReadLengthEncodedString(packet[pos:])
	if err != nil {
		return columnDef{}, err
	}
	pos += used
	_, used, _, err = protocol.ReadLengthEncodedString(packet[pos:])
	if err != nil {
		return columnDef{}, err
	}
	pos += used
	if len(packet) < pos+13 {
		return columnDef{name: string(nameBytes)}, io.ErrUnexpectedEOF
	}
	cd := columnDef{
		name:         string(nameBytes),
		charset:      binary.LittleEndian.Uint16(packet[pos+1 : pos+3]),
		columnLength: binary.LittleEndian.Uint32(packet[pos+3 : pos+7]),
		typ:          packet[pos+7],
		flags:        binary.LittleEndian.Uint16(packet[pos+8 : pos+10]),
		decimals:     packet[pos+10],
	}
	return cd, nil
}

func parseTextRow(packet []byte, columnCount int, types []byte) ([]driver.Value, error) {
	row := make([]driver.Value, columnCount)
	pos := 0
	for i := 0; i < columnCount; i++ {
		if pos >= len(packet) {
			return nil, io.ErrUnexpectedEOF
		}
		if packet[pos] == protocol.NullColumn {
			row[i] = nil
			pos++
			continue
		}
		raw, used, _, err := protocol.ReadLengthEncodedString(packet[pos:])
		if err != nil {
			return nil, err
		}
		pos += used
		typ := protocol.ColumnTypeVarString
		if i < len(types) {
			typ = types[i]
		}
		row[i] = textValue(raw, typ)
	}
	return row, nil
}

func textValue(raw []byte, typ byte) driver.Value {
	s := string(raw)
	switch typ {
	case protocol.ColumnTypeTiny, protocol.ColumnTypeShort, protocol.ColumnTypeLong, protocol.ColumnTypeInt24:
		if val, err := strconv.ParseInt(s, 10, 64); err == nil {
			return val
		}
		return s
	case protocol.ColumnTypeLongLong:
		if val, err := strconv.ParseInt(s, 10, 64); err == nil {
			return val
		}
		return s
	case protocol.ColumnTypeOracleNumberFloat:
		return s
	case protocol.ColumnTypeFloat, protocol.ColumnTypeDouble:
		if val, err := strconv.ParseFloat(s, 64); err == nil {
			return val
		}
	case protocol.ColumnTypeDecimal, protocol.ColumnTypeNewDecimal:
		return s
	case protocol.ColumnTypeDate, protocol.ColumnTypeDateTime, protocol.ColumnTypeTimestamp,
		protocol.ColumnTypeOracleTimestampNano, protocol.ColumnTypeOracleTimestampTZ, protocol.ColumnTypeOracleTimestampLTZ:
		formats := []string{
			"2006-01-02 15:04:05.999999999",
			"2006-01-02 15:04:05",
			"2006-01-02",
		}
		for _, f := range formats {
			if t, err := time.ParseInLocation(f, s, time.UTC); err == nil {
				return t
			}
		}
	case protocol.ColumnTypeTime:
		return s
	case protocol.ColumnTypeYear:
		if val, err := strconv.ParseInt(s, 10, 64); err == nil {
			return val
		}
		return s
	case protocol.ColumnTypeBit:
		return raw
	case protocol.ColumnTypeTinyBlob, protocol.ColumnTypeMediumBlob,
		protocol.ColumnTypeLongBlob, protocol.ColumnTypeBlob,
		protocol.ColumnTypeOracleRaw, protocol.ColumnTypeOracleBlob, protocol.ColumnTypeOracleClob:
		return raw
	case protocol.ColumnTypeOracleRowID, protocol.ColumnTypeOracleIntervalYM,
		protocol.ColumnTypeOracleIntervalDS, protocol.ColumnTypeOracleNVarChar2, protocol.ColumnTypeOracleNChar:
		return s
	}
	return s
}

func databaseTypeName(typ byte) string {
	switch typ {
	case protocol.ColumnTypeDecimal:
		return "DECIMAL"
	case protocol.ColumnTypeTiny:
		return "TINYINT"
	case protocol.ColumnTypeShort:
		return "SMALLINT"
	case protocol.ColumnTypeLong:
		return "INT"
	case protocol.ColumnTypeFloat:
		return "FLOAT"
	case protocol.ColumnTypeDouble:
		return "DOUBLE"
	case protocol.ColumnTypeNull:
		return "NULL"
	case protocol.ColumnTypeTimestamp:
		return "TIMESTAMP"
	case protocol.ColumnTypeLongLong:
		return "BIGINT"
	case protocol.ColumnTypeInt24:
		return "MEDIUMINT"
	case protocol.ColumnTypeDate:
		return "DATE"
	case protocol.ColumnTypeTime:
		return "TIME"
	case protocol.ColumnTypeDateTime:
		return "DATETIME"
	case protocol.ColumnTypeYear:
		return "YEAR"
	case protocol.ColumnTypeVarChar:
		return "VARCHAR"
	case protocol.ColumnTypeBit:
		return "BIT"
	case protocol.ColumnTypeJSON:
		return "JSON"
	case protocol.ColumnTypeNewDecimal:
		return "NEWDECIMAL"
	case protocol.ColumnTypeEnum:
		return "ENUM"
	case protocol.ColumnTypeSet:
		return "SET"
	case protocol.ColumnTypeTinyBlob:
		return "TINYBLOB"
	case protocol.ColumnTypeMediumBlob:
		return "MEDIUMBLOB"
	case protocol.ColumnTypeLongBlob:
		return "LONGBLOB"
	case protocol.ColumnTypeBlob:
		return "BLOB"
	case protocol.ColumnTypeVarString:
		return "VAR_STRING"
	case protocol.ColumnTypeString:
		return "STRING"
	case protocol.ColumnTypeGeometry:
		return "GEOMETRY"
	case protocol.ColumnTypeOracleTimestampTZ:
		return "TIMESTAMP WITH TIME ZONE"
	case protocol.ColumnTypeOracleTimestampLTZ:
		return "TIMESTAMP WITH LOCAL TIME ZONE"
	case protocol.ColumnTypeOracleRaw:
		return "RAW"
	case protocol.ColumnTypeOracleRowID:
		return "ROWID"
	case protocol.ColumnTypeOracleNumberFloat:
		return "NUMBER"
	case protocol.ColumnTypeOracleNVarChar2:
		return "NVARCHAR2"
	case protocol.ColumnTypeOracleNChar:
		return "NCHAR"
	case protocol.ColumnTypeOracleBlob:
		return "BLOB"
	case protocol.ColumnTypeOracleClob:
		return "CLOB"
	case protocol.ColumnTypeOracleIntervalYM:
		return "INTERVAL YEAR TO MONTH"
	case protocol.ColumnTypeOracleIntervalDS:
		return "INTERVAL DAY TO SECOND"
	default:
		return fmt.Sprintf("TYPE_%02X", typ)
	}
}

func scanType(typ byte) reflect.Type {
	switch typ {
	case protocol.ColumnTypeTiny, protocol.ColumnTypeShort, protocol.ColumnTypeLong, protocol.ColumnTypeInt24, protocol.ColumnTypeLongLong:
		return reflect.TypeOf(int64(0))
	case protocol.ColumnTypeFloat, protocol.ColumnTypeDouble:
		return reflect.TypeOf(float64(0))
	case protocol.ColumnTypeDecimal, protocol.ColumnTypeNewDecimal:
		return reflect.TypeOf("")
	case protocol.ColumnTypeDate, protocol.ColumnTypeDateTime, protocol.ColumnTypeTimestamp:
		return reflect.TypeOf(time.Time{})
	case protocol.ColumnTypeTinyBlob,
		protocol.ColumnTypeMediumBlob,
		protocol.ColumnTypeLongBlob,
		protocol.ColumnTypeBlob,
		protocol.ColumnTypeGeometry,
		protocol.ColumnTypeOracleRaw,
		protocol.ColumnTypeOracleBlob,
		protocol.ColumnTypeOracleClob:
		return reflect.TypeOf([]byte{})
	default:
		return reflect.TypeOf("")
	}
}
