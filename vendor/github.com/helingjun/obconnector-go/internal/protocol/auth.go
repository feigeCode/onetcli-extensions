package protocol

import (
	"crypto/sha1"
	"crypto/sha256"
)

func NativePasswordAuth(password string, seed []byte) []byte {
	if password == "" {
		return nil
	}

	hash1 := sha1.Sum([]byte(password))
	hash2 := sha1.Sum(hash1[:])

	h := sha1.New()
	_, _ = h.Write(seed)
	_, _ = h.Write(hash2[:])
	hash3 := h.Sum(nil)

	out := make([]byte, len(hash1))
	for i := range out {
		out[i] = hash1[i] ^ hash3[i]
	}
	return out
}

func CachingSha2PasswordAuth(password string, seed []byte) []byte {
	if password == "" {
		return nil
	}

	h1 := sha256.Sum256([]byte(password))
	h2 := sha256.Sum256(h1[:])

	h := sha256.New()
	_, _ = h.Write(h2[:])
	_, _ = h.Write(seed)
	h3 := h.Sum(nil)

	out := make([]byte, len(h1))
	for i := range out {
		out[i] = h1[i] ^ h3[i]
	}
	return out
}
