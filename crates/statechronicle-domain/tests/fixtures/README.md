# Independent v1 commit reference vector

`commit-ref-v1.bcs` was assembled without StateChronicle or a BCS library.
Its fields are the UTF-8 commit ID `cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W`,
the little-endian unsigned 64-bit sequence 918273, the UTF-8 digest
`sha256:` followed by 32 copies of `42`, and the 132-byte
`positive_v1_envelope_layout.bin` fixture from `statechronicle-core`.
Each string and the byte vector have a minimal ULEB128 length prefix.
The envelope fixture demonstrates framing and contains synthetic payload bytes.
