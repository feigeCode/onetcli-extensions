package protocol

import (
	"encoding/binary"
	"io"
)

const (
	OB20MagicNum         uint16 = 0x20AB
	OB20Version          uint16 = 20
	CompressHeaderLen    int    = 7
	OB20HeaderLen        int    = 24
	TotalHeaderLen       int    = 31 // CompressHeaderLen + OB20HeaderLen
	OB20TailLen          int    = 4
	OB20ExtraLengthField int    = 4
)

const (
	OB20FlagNone         uint32 = 0
	OB20FlagExtraInfo    uint32 = 1 << 0
	OB20FlagLast         uint32 = 1 << 1
	OB20FlagProxyReroute uint32 = 1 << 2
	OB20FlagNewExtraInfo uint32 = 1 << 3
)

const (
	OB20ExtraInfoTypeTraceID     uint16 = 2001
	OB20ExtraInfoTypeSessVar     uint16 = 2002
	OB20ExtraInfoTypeFullTrace   uint16 = 2003
	OB20ExtraInfoTypeTableID     uint16 = 2004
	OB20ExtraInfoTypePartitionID uint16 = 2005
)

// OB20Header represents the 31-byte OB 2.0 frame header.
type OB20Header struct {
	CompressLength   uint32 // 3 bytes LE
	CompressSeqNo    uint8
	UncompressLength uint32 // 3 bytes LE
	MagicNum         uint16 // 2 bytes LE (0x20AB)
	Version          uint16 // 2 bytes LE (20)
	ConnectionID     uint32 // 4 bytes LE
	RequestID        uint32 // 3 bytes LE
	PacketSeq        uint8
	PayloadLen       uint32 // 4 bytes LE
	Flag             uint32 // 4 bytes LE
	Reserved         uint16 // 2 bytes LE
	HeaderCRC        uint16 // 2 bytes LE
}

// Encode serializes the header into buf, calculating CRC16-IBM over the first 29 bytes.
func (h *OB20Header) Encode(buf []byte) {
	// --- Compress Header (7 bytes) ---
	buf[0] = byte(h.CompressLength)
	buf[1] = byte(h.CompressLength >> 8)
	buf[2] = byte(h.CompressLength >> 16)
	buf[3] = h.CompressSeqNo
	buf[4] = byte(h.UncompressLength)
	buf[5] = byte(h.UncompressLength >> 8)
	buf[6] = byte(h.UncompressLength >> 16)

	// --- OB20 Header (24 bytes) ---
	binary.LittleEndian.PutUint16(buf[7:9], h.MagicNum)
	binary.LittleEndian.PutUint16(buf[9:11], h.Version)
	binary.LittleEndian.PutUint32(buf[11:15], h.ConnectionID)
	buf[15] = byte(h.RequestID)
	buf[16] = byte(h.RequestID >> 8)
	buf[17] = byte(h.RequestID >> 16)
	buf[18] = h.PacketSeq
	binary.LittleEndian.PutUint32(buf[19:23], h.PayloadLen)
	binary.LittleEndian.PutUint32(buf[23:27], h.Flag)
	binary.LittleEndian.PutUint16(buf[27:29], h.Reserved)

	h.HeaderCRC = CRC16(buf[0:29])
	binary.LittleEndian.PutUint16(buf[29:31], h.HeaderCRC)
}

// Decode deserializes the header from buf and verifies CRC16-IBM, magic number, and version.
func (h *OB20Header) Decode(buf []byte) bool {
	if len(buf) < TotalHeaderLen {
		return false
	}
	h.CompressLength = uint32(buf[0]) | uint32(buf[1])<<8 | uint32(buf[2])<<16
	h.CompressSeqNo = buf[3]
	h.UncompressLength = uint32(buf[4]) | uint32(buf[5])<<8 | uint32(buf[6])<<16

	h.MagicNum = binary.LittleEndian.Uint16(buf[7:9])
	h.Version = binary.LittleEndian.Uint16(buf[9:11])
	if h.MagicNum != OB20MagicNum || h.Version != OB20Version {
		return false
	}
	h.ConnectionID = binary.LittleEndian.Uint32(buf[11:15])
	h.RequestID = uint32(buf[15]) | uint32(buf[16])<<8 | uint32(buf[17])<<16
	h.PacketSeq = buf[18]
	h.PayloadLen = binary.LittleEndian.Uint32(buf[19:23])
	h.Flag = binary.LittleEndian.Uint32(buf[23:27])
	h.Reserved = binary.LittleEndian.Uint16(buf[27:29])
	h.HeaderCRC = binary.LittleEndian.Uint16(buf[29:31])

	if h.HeaderCRC != 0 {
		return h.HeaderCRC == CRC16(buf[0:29])
	}
	return true
}

// PeekIsOB20 peeks at the buffer to check if it has a valid OB20 header format without verifying CRC.
func PeekIsOB20(buf []byte) bool {
	if len(buf) < TotalHeaderLen {
		return false
	}
	magicNum := binary.LittleEndian.Uint16(buf[7:9])
	version := binary.LittleEndian.Uint16(buf[9:11])
	return magicNum == OB20MagicNum && version == OB20Version
}

// CRC16-IBM table-based reflected checksum (poly 0x8005 reflected = 0xA001).
var crc16IBMTable = func() [256]uint16 {
	var table [256]uint16
	for i := 0; i < 256; i++ {
		crc := uint16(i)
		for j := 0; j < 8; j++ {
			if (crc & 1) != 0 {
				crc = (crc >> 1) ^ 0xA001
			} else {
				crc >>= 1
			}
		}
		table[i] = crc
	}
	return table
}()

// CRC16 calculates CRC16-IBM over data.
func CRC16(data []byte) uint16 {
	var crc uint16 = 0
	for _, b := range data {
		crc = (crc >> 8) ^ crc16IBMTable[(byte(crc)^b)&0xff]
	}
	return crc
}

// Castagnoli reflected CRC32C table (poly 0x82F63B78).
var crc32cOBTable = func() [256]uint32 {
	var table [256]uint32
	for i := 0; i < 256; i++ {
		crc := uint32(i)
		for j := 0; j < 8; j++ {
			if (crc & 1) != 0 {
				crc = (crc >> 1) ^ 0x82F63B78
			} else {
				crc >>= 1
			}
		}
		table[i] = crc
	}
	return table
}()

// OB20PayloadChecksum calculates Castagnoli reflected CRC32C with init=0 and no final XOR-out.
func OB20PayloadChecksum(data []byte) uint32 {
	var crc uint32 = 0
	for _, b := range data {
		crc = (crc >> 8) ^ crc32cOBTable[byte(crc)^b]
	}
	return crc
}

type OB20ExtraInfo struct {
	Type uint16
	Data []byte
}

func (e *OB20ExtraInfo) Encode(buf []byte) int {
	binary.LittleEndian.PutUint16(buf[0:2], e.Type)
	binary.LittleEndian.PutUint32(buf[2:6], uint32(len(e.Data)))
	copy(buf[6:], e.Data)
	return 6 + len(e.Data)
}

func (e *OB20ExtraInfo) TotalLen() int {
	return 6 + len(e.Data)
}

func ParseOB20ExtraInfo(data []byte) ([]OB20ExtraInfo, error) {
	var infos []OB20ExtraInfo
	pos := 0
	for pos < len(data) {
		if len(data[pos:]) < 6 {
			break
		}
		typ := binary.LittleEndian.Uint16(data[pos : pos+2])
		length := int(binary.LittleEndian.Uint32(data[pos+2 : pos+6]))
		pos += 6
		if len(data[pos:]) < length {
			return nil, io.ErrUnexpectedEOF
		}
		infos = append(infos, OB20ExtraInfo{
			Type: typ,
			Data: data[pos : pos+length],
		})
		pos += length
	}
	return infos, nil
}

func BuildFLTExtraInfo(traceID, spanID string) []byte {
	var buf []byte
	buf = append(buf, 0x01) // Tag TraceID
	buf = PutLengthEncodedString(buf, traceID)
	buf = append(buf, 0x02) // Tag SpanID
	buf = PutLengthEncodedString(buf, spanID)
	return buf
}
