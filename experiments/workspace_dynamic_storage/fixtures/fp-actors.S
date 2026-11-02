.arch armv8-a+simd
.section .text
 .global _start
_start:
 mov x19, x1
 mov x23, #0x50000000
 add x23, x23, #0x200
 add x24, x23, #0x20
 mov x4, #0x6766
 movk x4, #0x7461, lsl #16
 movk x4, #0x0065, lsl #32
 lsl x4, x4, #16
 mov x5, #0x2d00
 orr x4, x4, x5
 mov w5, #0x61
 mov x6, #0xb2b2
 cmp x19, x6
 b.ne 1f
 mov w5, #0x62
1: mov x6, #0xc3c3
 cmp x19, x6
 b.ne 2f
 mov w5, #0x63
2: orr x4, x4, x5
 str x4, [x23]
 umov x4, v0.d[0]
 cbnz x4, fail
 umov x4, v0.d[1]
 cbnz x4, fail
 umov x4, v1.d[0]
 cbnz x4, fail
 umov x4, v1.d[1]
 cbnz x4, fail
 umov x4, v2.d[0]
 cbnz x4, fail
 umov x4, v2.d[1]
 cbnz x4, fail
 umov x4, v3.d[0]
 cbnz x4, fail
 umov x4, v3.d[1]
 cbnz x4, fail
 umov x4, v4.d[0]
 cbnz x4, fail
 umov x4, v4.d[1]
 cbnz x4, fail
 umov x4, v5.d[0]
 cbnz x4, fail
 umov x4, v5.d[1]
 cbnz x4, fail
 umov x4, v6.d[0]
 cbnz x4, fail
 umov x4, v6.d[1]
 cbnz x4, fail
 umov x4, v7.d[0]
 cbnz x4, fail
 umov x4, v7.d[1]
 cbnz x4, fail
 umov x4, v8.d[0]
 cbnz x4, fail
 umov x4, v8.d[1]
 cbnz x4, fail
 umov x4, v9.d[0]
 cbnz x4, fail
 umov x4, v9.d[1]
 cbnz x4, fail
 umov x4, v10.d[0]
 cbnz x4, fail
 umov x4, v10.d[1]
 cbnz x4, fail
 umov x4, v11.d[0]
 cbnz x4, fail
 umov x4, v11.d[1]
 cbnz x4, fail
 umov x4, v12.d[0]
 cbnz x4, fail
 umov x4, v12.d[1]
 cbnz x4, fail
 umov x4, v13.d[0]
 cbnz x4, fail
 umov x4, v13.d[1]
 cbnz x4, fail
 umov x4, v14.d[0]
 cbnz x4, fail
 umov x4, v14.d[1]
 cbnz x4, fail
 umov x4, v15.d[0]
 cbnz x4, fail
 umov x4, v15.d[1]
 cbnz x4, fail
 umov x4, v16.d[0]
 cbnz x4, fail
 umov x4, v16.d[1]
 cbnz x4, fail
 umov x4, v17.d[0]
 cbnz x4, fail
 umov x4, v17.d[1]
 cbnz x4, fail
 umov x4, v18.d[0]
 cbnz x4, fail
 umov x4, v18.d[1]
 cbnz x4, fail
 umov x4, v19.d[0]
 cbnz x4, fail
 umov x4, v19.d[1]
 cbnz x4, fail
 umov x4, v20.d[0]
 cbnz x4, fail
 umov x4, v20.d[1]
 cbnz x4, fail
 umov x4, v21.d[0]
 cbnz x4, fail
 umov x4, v21.d[1]
 cbnz x4, fail
 umov x4, v22.d[0]
 cbnz x4, fail
 umov x4, v22.d[1]
 cbnz x4, fail
 umov x4, v23.d[0]
 cbnz x4, fail
 umov x4, v23.d[1]
 cbnz x4, fail
 umov x4, v24.d[0]
 cbnz x4, fail
 umov x4, v24.d[1]
 cbnz x4, fail
 umov x4, v25.d[0]
 cbnz x4, fail
 umov x4, v25.d[1]
 cbnz x4, fail
 umov x4, v26.d[0]
 cbnz x4, fail
 umov x4, v26.d[1]
 cbnz x4, fail
 umov x4, v27.d[0]
 cbnz x4, fail
 umov x4, v27.d[1]
 cbnz x4, fail
 umov x4, v28.d[0]
 cbnz x4, fail
 umov x4, v28.d[1]
 cbnz x4, fail
 umov x4, v29.d[0]
 cbnz x4, fail
 umov x4, v29.d[1]
 cbnz x4, fail
 umov x4, v30.d[0]
 cbnz x4, fail
 umov x4, v30.d[1]
 cbnz x4, fail
 umov x4, v31.d[0]
 cbnz x4, fail
 umov x4, v31.d[1]
 cbnz x4, fail
 mrs x4, fpcr
 cbnz x4, fail
 mrs x4, fpsr
 cbnz x4, fail
 mov x4, #4096
 eor x4, x4, x19
 fmov d0, x4
 mov x5, #8192
 eor x5, x5, x19
 ins v0.d[1], x5
 mov x4, #4097
 eor x4, x4, x19
 fmov d1, x4
 mov x5, #8193
 eor x5, x5, x19
 ins v1.d[1], x5
 mov x4, #4098
 eor x4, x4, x19
 fmov d2, x4
 mov x5, #8194
 eor x5, x5, x19
 ins v2.d[1], x5
 mov x4, #4099
 eor x4, x4, x19
 fmov d3, x4
 mov x5, #8195
 eor x5, x5, x19
 ins v3.d[1], x5
 mov x4, #4100
 eor x4, x4, x19
 fmov d4, x4
 mov x5, #8196
 eor x5, x5, x19
 ins v4.d[1], x5
 mov x4, #4101
 eor x4, x4, x19
 fmov d5, x4
 mov x5, #8197
 eor x5, x5, x19
 ins v5.d[1], x5
 mov x4, #4102
 eor x4, x4, x19
 fmov d6, x4
 mov x5, #8198
 eor x5, x5, x19
 ins v6.d[1], x5
 mov x4, #4103
 eor x4, x4, x19
 fmov d7, x4
 mov x5, #8199
 eor x5, x5, x19
 ins v7.d[1], x5
 mov x4, #4104
 eor x4, x4, x19
 fmov d8, x4
 mov x5, #8200
 eor x5, x5, x19
 ins v8.d[1], x5
 mov x4, #4105
 eor x4, x4, x19
 fmov d9, x4
 mov x5, #8201
 eor x5, x5, x19
 ins v9.d[1], x5
 mov x4, #4106
 eor x4, x4, x19
 fmov d10, x4
 mov x5, #8202
 eor x5, x5, x19
 ins v10.d[1], x5
 mov x4, #4107
 eor x4, x4, x19
 fmov d11, x4
 mov x5, #8203
 eor x5, x5, x19
 ins v11.d[1], x5
 mov x4, #4108
 eor x4, x4, x19
 fmov d12, x4
 mov x5, #8204
 eor x5, x5, x19
 ins v12.d[1], x5
 mov x4, #4109
 eor x4, x4, x19
 fmov d13, x4
 mov x5, #8205
 eor x5, x5, x19
 ins v13.d[1], x5
 mov x4, #4110
 eor x4, x4, x19
 fmov d14, x4
 mov x5, #8206
 eor x5, x5, x19
 ins v14.d[1], x5
 mov x4, #4111
 eor x4, x4, x19
 fmov d15, x4
 mov x5, #8207
 eor x5, x5, x19
 ins v15.d[1], x5
 mov x4, #4112
 eor x4, x4, x19
 fmov d16, x4
 mov x5, #8208
 eor x5, x5, x19
 ins v16.d[1], x5
 mov x4, #4113
 eor x4, x4, x19
 fmov d17, x4
 mov x5, #8209
 eor x5, x5, x19
 ins v17.d[1], x5
 mov x4, #4114
 eor x4, x4, x19
 fmov d18, x4
 mov x5, #8210
 eor x5, x5, x19
 ins v18.d[1], x5
 mov x4, #4115
 eor x4, x4, x19
 fmov d19, x4
 mov x5, #8211
 eor x5, x5, x19
 ins v19.d[1], x5
 mov x4, #4116
 eor x4, x4, x19
 fmov d20, x4
 mov x5, #8212
 eor x5, x5, x19
 ins v20.d[1], x5
 mov x4, #4117
 eor x4, x4, x19
 fmov d21, x4
 mov x5, #8213
 eor x5, x5, x19
 ins v21.d[1], x5
 mov x4, #4118
 eor x4, x4, x19
 fmov d22, x4
 mov x5, #8214
 eor x5, x5, x19
 ins v22.d[1], x5
 mov x4, #4119
 eor x4, x4, x19
 fmov d23, x4
 mov x5, #8215
 eor x5, x5, x19
 ins v23.d[1], x5
 mov x4, #4120
 eor x4, x4, x19
 fmov d24, x4
 mov x5, #8216
 eor x5, x5, x19
 ins v24.d[1], x5
 mov x4, #4121
 eor x4, x4, x19
 fmov d25, x4
 mov x5, #8217
 eor x5, x5, x19
 ins v25.d[1], x5
 mov x4, #4122
 eor x4, x4, x19
 fmov d26, x4
 mov x5, #8218
 eor x5, x5, x19
 ins v26.d[1], x5
 mov x4, #4123
 eor x4, x4, x19
 fmov d27, x4
 mov x5, #8219
 eor x5, x5, x19
 ins v27.d[1], x5
 mov x4, #4124
 eor x4, x4, x19
 fmov d28, x4
 mov x5, #8220
 eor x5, x5, x19
 ins v28.d[1], x5
 mov x4, #4125
 eor x4, x4, x19
 fmov d29, x4
 mov x5, #8221
 eor x5, x5, x19
 ins v29.d[1], x5
 mov x4, #4126
 eor x4, x4, x19
 fmov d30, x4
 mov x5, #8222
 eor x5, x5, x19
 ins v30.d[1], x5
 mov x4, #4127
 eor x4, x4, x19
 fmov d31, x4
 mov x5, #8223
 eor x5, x5, x19
 ins v31.d[1], x5
 mov x21, #0x400000
 mov x22, #0x08000000
 add x22, x22, #1
 mov x4, #0xb2b2
 cmp x19, x4
 b.ne 3f
 mov x21, #0x800000
 add x22, x22, #1
3: mov x4, #0xc3c3
 cmp x19, x4
 b.ne 4f
 mov x21, #0xc00000
 add x22, x22, #3
4: msr fpcr, x21
 msr fpsr, x22
 bl verify_fp
 mov w0, #0x72
 bl write_state
main_loop:
 bl read_state
 cmp w0, #0x72
 b.eq main_loop
 cmp w0, #0x73
 b.eq sender
 cmp w0, #0x77
 b.eq waiter
 cmp w0, #0x71
 b.eq success
 cmp w0, #0x64
 b.eq main_loop
 b fail
sender:
 mov x4, #0xa1a1
 cmp x19, x4
 b.ne fail
 svc #0x160
 bl verify_fp
 cmp x0, #1
 b.ne fail
 mov x25, x0
 mov x26, x1
 mov x27, x2
 mov w4, #0x57
 strb w4, [x24, #1]
 mov x0, x25
 mov x1, x26
 add x2, x24, #1
 mov x3, #1
 mov x4, x27
 svc #0x161
 bl verify_fp
 cbnz x0, fail
 mov w0, #0x64
 bl write_state
 b main_loop
waiter:
 mov x4, #0xb2b2
 cmp x19, x4
 b.ne fail
 svc #0x160
 bl verify_fp
 cbnz x0, fail
 mov x25, x0
 mov x26, x1
 mov x27, x2
 mov x0, x27
 svc #0x163
 bl verify_fp
 cbnz x0, fail
 add x0, x24, #1
 mov x1, #1
 mov x2, x27
 svc #0x162
 bl verify_fp
 cmp x0, #1
 b.ne fail
 cmp x1, x25
 b.ne fail
 cmp x2, x26
 b.ne fail
 ldrb w4, [x24, #1]
 cmp w4, #0x57
 b.ne fail
 mov w0, #0x64
 bl write_state
 b main_loop
write_state:
 mov x28, x30
 strb w0, [x24]
write_retry:
 mov x0, x23
 mov x1, #7
 mov x2, x24
 mov x3, #1
 svc #0x141
 bl verify_fp
 cmn x0, #11
 b.eq write_retry
 cmp x0, #1
 b.ne fail
 br x28
read_state:
 mov x28, x30
read_retry:
 mov x0, x23
 mov x1, #7
 mov x2, x24
 mov x3, #1
 svc #0x140
 bl verify_fp
 cmn x0, #11
 b.eq read_retry
 cmp x0, #1
 b.ne fail
 ldrb w0, [x24]
 br x28
verify_fp:
 mrs x4, fpcr
 cmp x4, x21
 b.ne fail
 mrs x4, fpsr
 cmp x4, x22
 b.ne fail
 umov x4, v0.d[0]
 mov x5, #4096
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v0.d[1]
 mov x5, #8192
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v1.d[0]
 mov x5, #4097
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v1.d[1]
 mov x5, #8193
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v2.d[0]
 mov x5, #4098
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v2.d[1]
 mov x5, #8194
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v3.d[0]
 mov x5, #4099
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v3.d[1]
 mov x5, #8195
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v4.d[0]
 mov x5, #4100
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v4.d[1]
 mov x5, #8196
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v5.d[0]
 mov x5, #4101
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v5.d[1]
 mov x5, #8197
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v6.d[0]
 mov x5, #4102
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v6.d[1]
 mov x5, #8198
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v7.d[0]
 mov x5, #4103
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v7.d[1]
 mov x5, #8199
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v8.d[0]
 mov x5, #4104
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v8.d[1]
 mov x5, #8200
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v9.d[0]
 mov x5, #4105
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v9.d[1]
 mov x5, #8201
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v10.d[0]
 mov x5, #4106
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v10.d[1]
 mov x5, #8202
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v11.d[0]
 mov x5, #4107
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v11.d[1]
 mov x5, #8203
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v12.d[0]
 mov x5, #4108
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v12.d[1]
 mov x5, #8204
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v13.d[0]
 mov x5, #4109
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v13.d[1]
 mov x5, #8205
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v14.d[0]
 mov x5, #4110
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v14.d[1]
 mov x5, #8206
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v15.d[0]
 mov x5, #4111
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v15.d[1]
 mov x5, #8207
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v16.d[0]
 mov x5, #4112
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v16.d[1]
 mov x5, #8208
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v17.d[0]
 mov x5, #4113
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v17.d[1]
 mov x5, #8209
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v18.d[0]
 mov x5, #4114
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v18.d[1]
 mov x5, #8210
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v19.d[0]
 mov x5, #4115
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v19.d[1]
 mov x5, #8211
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v20.d[0]
 mov x5, #4116
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v20.d[1]
 mov x5, #8212
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v21.d[0]
 mov x5, #4117
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v21.d[1]
 mov x5, #8213
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v22.d[0]
 mov x5, #4118
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v22.d[1]
 mov x5, #8214
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v23.d[0]
 mov x5, #4119
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v23.d[1]
 mov x5, #8215
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v24.d[0]
 mov x5, #4120
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v24.d[1]
 mov x5, #8216
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v25.d[0]
 mov x5, #4121
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v25.d[1]
 mov x5, #8217
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v26.d[0]
 mov x5, #4122
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v26.d[1]
 mov x5, #8218
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v27.d[0]
 mov x5, #4123
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v27.d[1]
 mov x5, #8219
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v28.d[0]
 mov x5, #4124
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v28.d[1]
 mov x5, #8220
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v29.d[0]
 mov x5, #4125
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v29.d[1]
 mov x5, #8221
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v30.d[0]
 mov x5, #4126
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v30.d[1]
 mov x5, #8222
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v31.d[0]
 mov x5, #4127
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 umov x4, v31.d[1]
 mov x5, #8223
 eor x5, x5, x19
 cmp x4, x5
 b.ne fail
 ret
success:
 bl verify_fp
 mov x0, #0
 svc #0x123
fail:
 mov x0, #42
 svc #0x123
 .section .data
 .quad 0x1122334455667788
 .section .bss
 .space 32
