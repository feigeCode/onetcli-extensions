package protocol

import (
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"strings"
)

// SeqError is returned when a packet's sequence number doesn't match expectations.
// It implements net.Error so that the driver marks the connection as bad.
type SeqError struct {
	Got  byte
	Want byte
}

func (e *SeqError) Error() string {
	return fmt.Sprintf("unexpected packet sequence: got %d, want %d", e.Got, e.Want)
}

func (e *SeqError) Timeout() bool   { return false }
func (e *SeqError) Temporary() bool { return false }

var _ net.Error = (*SeqError)(nil)

const maxPayloadLen = 1<<24 - 1
const maxOB20Chunk = maxPayloadLen - OB20HeaderLen - OB20TailLen - 4

type PacketConn struct {
	rw               io.ReadWriter
	seq              byte
	ob20             bool
	ob20Magic        uint16
	connectionID     uint32
	requestID        uint32
	extraInfos       []OB20ExtraInfo
	mysqlBuf         []byte
	ob20NewExtraInfo bool
	ob20Checksum     bool
	traceWriter      io.Writer
	compressed       bool
	compressBuf      bytes.Buffer
}

func NewPacketConn(rw io.ReadWriter) *PacketConn {
	return &PacketConn{rw: rw}
}

func (c *PacketConn) SetTraceWriter(w io.Writer) {
	c.traceWriter = w
}

func (c *PacketConn) ResetSequence() {
	c.seq = 0
}

func (c *PacketConn) EnableOB20(connectionID uint32, magic uint16, newExtraInfo bool, disableChecksum bool) {
	c.ob20 = true
	c.ob20Magic = magic
	c.connectionID = connectionID
	c.ob20NewExtraInfo = newExtraInfo
	c.ob20Checksum = !disableChecksum
}

func (c *PacketConn) AddExtraInfo(typ uint16, data []byte) {
	c.extraInfos = append(c.extraInfos, OB20ExtraInfo{Type: typ, Data: data})
}

func (c *PacketConn) ClearExtraInfo() {
	c.extraInfos = nil
}

func (c *PacketConn) NextRequest() {
	c.requestID++
}

func (c *PacketConn) ReadPacket() ([]byte, error) {
	var out []byte
	for {
		if c.ob20 {
			if len(c.mysqlBuf) >= 4 {
				mysqlLen := int(c.mysqlBuf[0]) | int(c.mysqlBuf[1])<<8 | int(c.mysqlBuf[2])<<16
				if len(c.mysqlBuf) >= 4+mysqlLen {
					payload := c.mysqlBuf[4 : 4+mysqlLen]
					out = append(out, payload...)
					c.mysqlBuf = c.mysqlBuf[4+mysqlLen:]

					if mysqlLen < maxPayloadLen {
						return out, nil
					}
					continue
				}
			}

			var obHeader [TotalHeaderLen]byte
			if _, err := io.ReadFull(c.rw, obHeader[:]); err != nil {
				c.dumpRawRX("read OB20 header failed", obHeader[:], err)
				return nil, err
			}
			var h OB20Header
			if !h.Decode(obHeader[:]) {
				c.dumpRawRX("decode OB20 header failed", obHeader[:], nil)
				return nil, fmt.Errorf("invalid OB 2.0 header")
			}

			// Validate compressLength consistency
			expectedCompressLen := uint32(OB20HeaderLen + int(h.PayloadLen) + OB20TailLen)
			if h.CompressLength != expectedCompressLen {
				return nil, fmt.Errorf("OB20 packet length mismatch: compressLength=%d, expected=%d (payload=%d)",
					h.CompressLength, expectedCompressLen, h.PayloadLen)
			}
			if h.UncompressLength != 0 {
				return nil, fmt.Errorf("OB20 unexpected uncompress length: %d", h.UncompressLength)
			}

			c.traceOB20RX("OB20 frame: magic=0x%04x version=%d connID=%d reqID=%d seq=%d payload=%d flags=%s",
				h.MagicNum, h.Version, h.ConnectionID, h.RequestID,
				h.PacketSeq, h.PayloadLen, ob20FlagNames(h.Flag))

			obPayload := make([]byte, h.PayloadLen)
			if _, err := io.ReadFull(c.rw, obPayload); err != nil {
				return nil, err
			}

			var obTrailer [4]byte
			if _, err := io.ReadFull(c.rw, obTrailer[:]); err != nil {
				return nil, err
			}

			expectedChecksum := binary.LittleEndian.Uint32(obTrailer[:])
			if expectedChecksum != 0 && OB20PayloadChecksum(obPayload) != expectedChecksum {
				return nil, fmt.Errorf("invalid OB 2.0 payload checksum: expected 0x%08x, got 0x%08x", expectedChecksum, OB20PayloadChecksum(obPayload))
			}

			mysqlData := obPayload
			if h.Flag&OB20FlagExtraInfo != 0 {
				if len(mysqlData) < OB20ExtraLengthField {
					return nil, fmt.Errorf("truncated extra_length in OB20 payload")
				}
				extraLength := binary.LittleEndian.Uint32(mysqlData[0:4])
				extraTotal := OB20ExtraLengthField + int(extraLength)
				if len(mysqlData) < extraTotal {
					return nil, fmt.Errorf("truncated extra_info: declared %d bytes, payload has %d", extraTotal, len(mysqlData))
				}
				mysqlData = mysqlData[extraTotal:]
			}

			c.mysqlBuf = append(c.mysqlBuf, mysqlData...)

		} else if c.compressed {
			var compressedHeader [7]byte
			if _, err := io.ReadFull(c.rw, compressedHeader[:]); err != nil {
				return nil, err
			}
			compressLen := int(compressedHeader[0]) | int(compressedHeader[1])<<8 | int(compressedHeader[2])<<16
			_ = compressedHeader[3] // compressed sequence
			uncompressLen := int(compressedHeader[4]) | int(compressedHeader[5])<<8 | int(compressedHeader[6])<<16

			compressedPayload := make([]byte, compressLen)
			if _, err := io.ReadFull(c.rw, compressedPayload); err != nil {
				return nil, err
			}

			if uncompressLen == 0 {
				out = append(out, compressedPayload...)
				return out, nil
			}

			reader, err := zlib.NewReader(bytes.NewReader(compressedPayload))
			if err != nil {
				return nil, fmt.Errorf("zlib decompression failed: %w", err)
			}
			decompressed := make([]byte, uncompressLen)
			if _, err := io.ReadFull(reader, decompressed); err != nil {
				_ = reader.Close()
				return nil, fmt.Errorf("zlib read failed: %w", err)
			}
			_ = reader.Close()

			c.mysqlBuf = append(c.mysqlBuf, decompressed...)
			for len(c.mysqlBuf) >= 4 {
				mysqlLen := int(c.mysqlBuf[0]) | int(c.mysqlBuf[1])<<8 | int(c.mysqlBuf[2])<<16
				if len(c.mysqlBuf) < 4+mysqlLen {
					break
				}
				payload := c.mysqlBuf[4 : 4+mysqlLen]
				out = append(out, payload...)
				c.mysqlBuf = c.mysqlBuf[4+mysqlLen:]
				c.seq++
				if mysqlLen < maxPayloadLen {
					return out, nil
				}
			}
			if len(out) > 0 {
				return out, nil
			}
		} else {
			var header [4]byte
			if _, err := io.ReadFull(c.rw, header[:]); err != nil {
				return nil, err
			}

			payloadLen := int(header[0]) | int(header[1])<<8 | int(header[2])<<16
			gotSeq := header[3]
			if gotSeq != c.seq {
				return nil, &SeqError{Got: gotSeq, Want: c.seq}
			}
			c.seq++

			payload := make([]byte, payloadLen)
			if _, err := io.ReadFull(c.rw, payload); err != nil {
				return nil, err
			}
			out = append(out, payload...)
			if payloadLen < maxPayloadLen {
				return out, nil
			}
		}
	}
}

func (c *PacketConn) WritePacket(payload []byte) error {
	for {
		chunkLen := len(payload)
		chunkLimit := maxPayloadLen
		if c.ob20 {
			chunkLimit = maxOB20Chunk
		}
		if chunkLen > chunkLimit {
			chunkLen = chunkLimit
		}

		mysqlLen := 4 + chunkLen
		extraLen := 0
		if c.ob20 {
			for _, info := range c.extraInfos {
				extraLen += info.TotalLen()
			}
		}

		var payloadBuf []byte
		if c.ob20 && extraLen > 0 {
			payloadBuf = make([]byte, 4+extraLen+mysqlLen)
			binary.LittleEndian.PutUint32(payloadBuf[0:4], uint32(extraLen))
			pos := 4
			for _, info := range c.extraInfos {
				pos += info.Encode(payloadBuf[pos:])
			}
			mysqlPos := 4 + extraLen
			payloadBuf[mysqlPos] = byte(chunkLen)
			payloadBuf[mysqlPos+1] = byte(chunkLen >> 8)
			payloadBuf[mysqlPos+2] = byte(chunkLen >> 16)
			payloadBuf[mysqlPos+3] = c.seq
			copy(payloadBuf[mysqlPos+4:], payload[:chunkLen])
		} else {
			payloadBuf = make([]byte, mysqlLen)
			payloadBuf[0] = byte(chunkLen)
			payloadBuf[1] = byte(chunkLen >> 8)
			payloadBuf[2] = byte(chunkLen >> 16)
			payloadBuf[3] = c.seq
			copy(payloadBuf[4:], payload[:chunkLen])
		}
		c.seq++

		if c.ob20 {
			payloadLen := uint32(len(payloadBuf))
			compressLength := uint32(TotalHeaderLen - CompressHeaderLen + len(payloadBuf) + OB20TailLen)

			flag := OB20FlagLast
			if extraLen > 0 {
				flag |= OB20FlagExtraInfo
			}
			if c.ob20NewExtraInfo {
				flag |= OB20FlagNewExtraInfo
			}

			h := OB20Header{
				CompressLength:   compressLength,
				CompressSeqNo:    c.seq - 1,
				UncompressLength: 0,
				MagicNum:         c.ob20Magic,
				Version:          OB20Version,
				ConnectionID:     c.connectionID,
				RequestID:        c.requestID & 0x00FFFFFF,
				PacketSeq:        c.seq - 1,
				PayloadLen:       payloadLen,
				Flag:             flag,
				Reserved:         0,
			}

			var obHeaderBuf [TotalHeaderLen]byte
			h.Encode(obHeaderBuf[:])
			if !c.ob20Checksum {
				obHeaderBuf[TotalHeaderLen-2] = 0
				obHeaderBuf[TotalHeaderLen-1] = 0
			}

			if c.traceWriter != nil {
				fullFrame := make([]byte, 0, TotalHeaderLen+len(payloadBuf)+OB20TailLen)
				fullFrame = append(fullFrame, obHeaderBuf[:]...)
				fullFrame = append(fullFrame, payloadBuf...)
				tail := OB20PayloadChecksum(payloadBuf)
				if !c.ob20Checksum {
					tail = 0
				}
				var tailBuf [4]byte
				binary.LittleEndian.PutUint32(tailBuf[:], tail)
				fullFrame = append(fullFrame, tailBuf[:]...)
				fmt.Fprintf(c.traceWriter, "obconnector-go: tx: OB20 frame len=%d header=%s payload=%d extra=%d flag=%s\n",
					len(fullFrame), hex.EncodeToString(obHeaderBuf[:]), payloadLen, extraLen, ob20FlagNames(flag))
				fmt.Fprintf(c.traceWriter, "obconnector-go: tx: full hex: %s\n", hex.EncodeToString(fullFrame))
			}

			if _, err := c.rw.Write(obHeaderBuf[:]); err != nil {
				return err
			}
			if _, err := c.rw.Write(payloadBuf); err != nil {
				return err
			}

			tail := OB20PayloadChecksum(payloadBuf)
			if !c.ob20Checksum {
				tail = 0
			}
			var obTrailer [4]byte
			binary.LittleEndian.PutUint32(obTrailer[:], tail)
			if _, err := c.rw.Write(obTrailer[:]); err != nil {
				return err
			}
		} else if c.compressed && len(payloadBuf) > 50 {
			c.compressBuf.Reset()
			writer := zlib.NewWriter(&c.compressBuf)
			if _, err := writer.Write(payloadBuf); err != nil {
				_ = writer.Close()
				return err
			}
			writer.Close()

			compressedPayload := c.compressBuf.Bytes()
			uncompressLen := uint32(len(payloadBuf))

			if len(compressedPayload) < len(payloadBuf) {
				var compressedHeader [7]byte
				compressedHeader[0] = byte(len(compressedPayload))
				compressedHeader[1] = byte(len(compressedPayload) >> 8)
				compressedHeader[2] = byte(len(compressedPayload) >> 16)
				compressedHeader[3] = c.seq - 1
				compressedHeader[4] = byte(uncompressLen)
				compressedHeader[5] = byte(uncompressLen >> 8)
				compressedHeader[6] = byte(uncompressLen >> 16)

				if _, err := c.rw.Write(compressedHeader[:]); err != nil {
					return err
				}
				if _, err := c.rw.Write(compressedPayload); err != nil {
					return err
				}
			} else {
				var compressedHeader [7]byte
				compressedHeader[0] = byte(len(payloadBuf))
				compressedHeader[1] = byte(len(payloadBuf) >> 8)
				compressedHeader[2] = byte(len(payloadBuf) >> 16)
				compressedHeader[3] = c.seq - 1
				compressedHeader[4] = 0
				compressedHeader[5] = 0
				compressedHeader[6] = 0

				if _, err := c.rw.Write(compressedHeader[:]); err != nil {
					return err
				}
				if _, err := c.rw.Write(payloadBuf); err != nil {
					return err
				}
			}
		} else {
			if _, err := c.rw.Write(payloadBuf); err != nil {
				return err
			}
		}

		payload = payload[chunkLen:]
		if chunkLen < chunkLimit {
			return nil
		}
		if len(payload) == 0 {
			return c.writeEmptyContinuation()
		}
	}
}

func (c *PacketConn) writeEmptyContinuation() error {
	mysqlHeader := make([]byte, 4)
	mysqlHeader[0] = 0
	mysqlHeader[1] = 0
	mysqlHeader[2] = 0
	mysqlHeader[3] = c.seq
	c.seq++

	if c.ob20 {
		payloadLen := uint32(len(mysqlHeader))
		compressLength := uint32(TotalHeaderLen - CompressHeaderLen + len(mysqlHeader) + OB20TailLen) // 24 + 4 + 4 = 32

		h := OB20Header{
			CompressLength:   compressLength,
			CompressSeqNo:    mysqlHeader[3],
			UncompressLength: 0,
			MagicNum:         c.ob20Magic,
			Version:          OB20Version,
			ConnectionID:     c.connectionID,
			RequestID:        c.requestID & 0x00FFFFFF,
			PacketSeq:        mysqlHeader[3],
			PayloadLen:       payloadLen,
			Flag:             OB20FlagLast,
			Reserved:         0,
		}
		if c.ob20NewExtraInfo {
			h.Flag |= OB20FlagNewExtraInfo
		}

		var obHeaderBuf [TotalHeaderLen]byte
		h.Encode(obHeaderBuf[:])
		if !c.ob20Checksum {
			obHeaderBuf[TotalHeaderLen-2] = 0
			obHeaderBuf[TotalHeaderLen-1] = 0
		}

		if _, err := c.rw.Write(obHeaderBuf[:]); err != nil {
			return err
		}
		if _, err := c.rw.Write(mysqlHeader); err != nil {
			return err
		}

		tail := OB20PayloadChecksum(mysqlHeader)
		if !c.ob20Checksum {
			tail = 0
		}
		var obTrailer [4]byte
		binary.LittleEndian.PutUint32(obTrailer[:], tail)
		_, err := c.rw.Write(obTrailer[:])
		return err
	}

	_, err := c.rw.Write(mysqlHeader)
	return err
}

func (c *PacketConn) IsOB20() bool {
	return c.ob20
}

func (c *PacketConn) ConnectionID() uint32 {
	return c.connectionID
}

func (c *PacketConn) EnableCompression() {
	c.compressed = true
}

func (c *PacketConn) traceOB20RX(format string, args ...any) {
	if c.traceWriter == nil {
		return
	}
	fmt.Fprintf(c.traceWriter, "obconnector-go: rx: "+format+"\n", args...)
}

func (c *PacketConn) dumpRawRX(reason string, peek []byte, err error) {
	if c.traceWriter == nil {
		return
	}
	n := len(peek)
	if n > 64 {
		n = 64
	}
	fmt.Fprintf(c.traceWriter, "obconnector-go: rx: %s err=%v first=%d bytes: %s\n",
		reason, err, len(peek), hex.EncodeToString(peek[:n]))
}

func ob20FlagNames(flag uint32) string {
	parts := []string{}
	if flag&OB20FlagExtraInfo != 0 {
		parts = append(parts, "EXTRA_INFO")
	}
	if flag&OB20FlagLast != 0 {
		parts = append(parts, "LAST")
	}
	if flag&OB20FlagProxyReroute != 0 {
		parts = append(parts, "PROXY_REROUTE")
	}
	if flag&OB20FlagNewExtraInfo != 0 {
		parts = append(parts, "NEW_EXTRA_INFO")
	}
	if len(parts) == 0 {
		return "NONE"
	}
	return strings.Join(parts, "|")
}
