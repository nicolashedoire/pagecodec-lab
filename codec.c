/*
 * Experimental adaptive page codec, independently implemented for this bench.
 * The techniques (base/delta, bit packing, uniform and raw modes) are known;
 * this implementation makes no claim of a new compression invention.
 * Wire format is little endian. n is supplied by the caller, not in the frame.
 * 0 RAW:          tag, n original bytes
 * 1 BYTE_UNIFORM: tag, repeated byte
 * 2 REPEAT64:     tag, eight-byte repeating pattern (including partial tail)
 * 3 BYTE_DELTA4:  tag, minimum byte, packed low-first four-bit residuals
 * 4 LANES32:     tag, four bit widths, four uint32 minima, lane-major residuals,
 *                final n%4 raw bytes. Words are interleaved in four lanes.
 * 5 SPARSE32:    tag, uint32 base, uint16 exception count, then ordered records
 *                (uint16 word index,uint32 value), final n%4 raw bytes.
 */
#include "codec.h"
#include <limits.h>
#include <string.h>

enum { RAW, BYTE_UNIFORM, REPEAT64, BYTE_DELTA4, LANES32, SPARSE32 };

static uint32_t load32(const uint8_t *p) {
    uint32_t x;
    memcpy(&x, p, sizeof x);
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
    x = __builtin_bswap32(x);
#endif
    return x;
}
static void store32(uint8_t *p, uint32_t x) {
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
    x = __builtin_bswap32(x);
#endif
    memcpy(p, &x, sizeof x);
}
static unsigned load16(const uint8_t *p) {
    return (unsigned)p[0] | ((unsigned)p[1]<<8);
}
static void store16(uint8_t *p,unsigned x) {
    p[0]=(uint8_t)x; p[1]=(uint8_t)(x>>8);
}

/* The fixed prefix probe may miss compressible inputs, but never loses data.
 * Its purpose is to avoid a complete exception scan on ordinary pages. */
static size_t sparse_size(uint8_t *dst,size_t cap,const uint8_t *src,size_t n,
                          uint32_t *base,unsigned *exceptions) {
    size_t words=n/4;
    if (words<2 || words>65536) return SIZE_MAX;
    *base=load32(src);
    if (words>=3) {
        uint32_t a=load32(src+4),b=load32(src+8);
        if (a==b) *base=a;
    }
    size_t sample=words<32?words:32, equal=0;
    for (size_t i=0;i<sample;++i) equal+=load32(src+4*i)==*base;
    if (equal*4<sample*3) return SIZE_MAX;
    size_t limit=(4*words-7)/6;
    unsigned count=0;
    for (size_t i=0;i<words;++i) {
        uint32_t value=load32(src+4*i);
        if (value!=*base) {
            ++count;
            if (count>limit) return SIZE_MAX;
            /* Emit candidate records during this scan. Other modes may
             * overwrite them; all writes stay inside caller capacity. */
            size_t position=7+6*(size_t)(count-1);
            if (position<=cap && cap-position>=6) {
                store16(dst+position,(unsigned)i);
                store32(dst+position+2,value);
            }
        }
    }
    *exceptions=count;
    return 7+6*(size_t)count+n%4;
}
static void write_sparse(uint8_t *dst,const uint8_t *src,size_t n,
                         uint32_t base,unsigned exceptions) {
    dst[0]=SPARSE32; store32(dst+1,base); store16(dst+5,exceptions);
    /* Exception records were emitted by sparse_size, avoiding a second scan. */
    uint8_t *cursor=dst+7+6*(size_t)exceptions;
    memcpy(cursor,src+(n/4)*4,n%4);
}
static unsigned width_for(uint32_t delta) {
    if (!delta) return 0;
    unsigned bits=32u-(unsigned)__builtin_clz(delta);
    return (bits+3u)&~3u;
}
static bool valid_width(unsigned width) {
    return width<=32 && width%4==0;
}
static size_t lane_count(size_t words, unsigned lane) {
    return words / 4 + (lane < words % 4);
}
static size_t packed_size(size_t count, unsigned width) {
    return width%8 ? (count/2)*(width/4)+(count%2)*((width+7)/8)
                   : count*(width/8);
}
static size_t lanes_size(size_t n, const unsigned width[4]) {
    size_t total = 21;
    for (unsigned lane = 0; lane < 4; ++lane) {
        size_t amount = packed_size(lane_count(n / 4, lane), width[lane]);
        if (amount > SIZE_MAX - total) return SIZE_MAX;
        total += amount;
    }
    if (n % 4 > SIZE_MAX - total) return SIZE_MAX;
    return total + n % 4;
}

size_t rc_bound(size_t n) { return n == SIZE_MAX ? 0 : n + 1; }

/* Four independent minima permit fixed-field 16-byte records and ordinary
 * uint32 arrays to use small residual widths without a dictionary search. */
static void extrema(const uint8_t *src, size_t n, uint32_t lo[4], uint32_t hi[4]) {
    uint32_t negative_min[4],positive_max[4];
    for (unsigned lane=0;lane<4;++lane) {
        lo[lane]=negative_min[lane]=UINT32_MAX;
        hi[lane]=positive_max[lane]=0;
    }
    for (size_t i=0;i<n/4;++i) {
        unsigned lane=(unsigned)(i%4);
        uint32_t x=load32(src+4*i);
        if (x<lo[lane]) lo[lane]=x;
        if (x>hi[lane]) hi[lane]=x;
        if (x&UINT32_C(0x80000000)) {
            if (x<negative_min[lane]) negative_min[lane]=x;
        } else if (x>positive_max[lane]) positive_max[lane]=x;
    }
    for (unsigned lane=0;lane<4;++lane) {
        if (!lane_count(n/4,lane)) { lo[lane]=hi[lane]=0; continue; }
        if (lo[lane]<UINT32_C(0x80000000) && hi[lane]>=UINT32_C(0x80000000)) {
            /* A signed interval crossing zero is an unsigned interval that
             * wraps at 2^32. Modular subtraction/addition needs no mode flag. */
            uint32_t span=positive_max[lane]-negative_min[lane];
            if (span<hi[lane]-lo[lane]) {
                lo[lane]=negative_min[lane];
                hi[lane]=positive_max[lane];
            }
        }
    }
}

static void pack_nibbles(uint8_t *dst,const uint8_t *src,size_t count,
                         uint32_t base,unsigned whole) {
    const unsigned stride=2*whole+1;
    for (size_t i=0;i<count;i+=2) {
        uint32_t a=load32(src+16*i)-base;
        uint32_t b=i+1<count ? load32(src+16*(i+1))-base : 0;
        uint8_t *p=dst+(i/2)*stride;
        for (unsigned j=0;j<whole;++j) p[j]=(uint8_t)(a>>(8*j));
        p[whole]=(uint8_t)((a>>(8*whole))|(b<<4));
        if (i+1<count)
            for (unsigned j=0;j<whole;++j) p[whole+1+j]=(uint8_t)(b>>(4+8*j));
    }
}
static void unpack_nibbles(uint8_t *dst,const uint8_t *src,size_t count,
                           uint32_t base,unsigned whole) {
    const unsigned stride=2*whole+1;
    for (size_t i=0;i<count;i+=2) {
        const uint8_t *p=src+(i/2)*stride;
        uint32_t a=(uint32_t)(p[whole]&15)<<(8*whole);
        for (unsigned j=0;j<whole;++j) a|=(uint32_t)p[j]<<(8*j);
        store32(dst+16*i,base+a);
        if (i+1<count) {
            uint32_t b=p[whole]>>4;
            for (unsigned j=0;j<whole;++j) b|=(uint32_t)p[whole+1+j]<<(4+8*j);
            store32(dst+16*(i+1),base+b);
        }
    }
}

static void pack_lane(uint8_t *dst, const uint8_t *src, size_t count,
                      unsigned width, uint32_t base) {
    if (width == 0) return;
    if (width%8) {
        pack_nibbles(dst,src,count,base,width/8);
    } else if (width == 8) {
        for (size_t i=0; i<count; ++i) dst[i]=(uint8_t)(load32(src+16*i)-base);
    } else if (width == 16) {
        for (size_t i=0; i<count; ++i) {
            uint32_t v=load32(src+16*i)-base;
            dst[2*i]=(uint8_t)v; dst[2*i+1]=(uint8_t)(v>>8);
        }
    } else if (width == 24) {
        for (size_t i=0; i<count; ++i) {
            uint32_t v=load32(src+16*i)-base;
            dst[3*i]=(uint8_t)v; dst[3*i+1]=(uint8_t)(v>>8); dst[3*i+2]=(uint8_t)(v>>16);
        }
    } else {
        for (size_t i=0; i<count; ++i) store32(dst+4*i, load32(src+16*i)-base);
    }
}

static void unpack_lane(uint8_t *dst, const uint8_t *src, size_t count,
                        unsigned width, uint32_t base) {
    if (width == 0) {
        for (size_t i=0; i<count; ++i) store32(dst+16*i, base);
    } else if (width%8) {
        unpack_nibbles(dst,src,count,base,width/8);
    } else if (width == 8) {
        for (size_t i=0; i<count; ++i) store32(dst+16*i, base+src[i]);
    } else if (width == 16) {
        for (size_t i=0; i<count; ++i)
            store32(dst+16*i, base + (uint32_t)src[2*i] + ((uint32_t)src[2*i+1]<<8));
    } else if (width == 24) {
        for (size_t i=0; i<count; ++i)
            store32(dst+16*i, base + (uint32_t)src[3*i] + ((uint32_t)src[3*i+1]<<8) + ((uint32_t)src[3*i+2]<<16));
    } else {
        for (size_t i=0; i<count; ++i) store32(dst+16*i, base+load32(src+4*i));
    }
}

size_t rc_encode(uint8_t *dst, size_t cap, const uint8_t *src, size_t n) {
    if (!dst || (!src && n) || !rc_bound(n) || !cap) return 0;
    if (!n) { dst[0]=RAW; return 1; }
    uint8_t low=255, high=0;
    for (size_t i=0; i<n; ++i) {
        if (src[i]<low) low=src[i];
        if (src[i]>high) high=src[i];
    }
    if (low==high && n>=2) {
        if (cap<2) return 0;
        dst[0]=BYTE_UNIFORM; dst[1]=low; return 2;
    }
    uint32_t sparse_base=0;
    unsigned exception_count=0;
    size_t sparse_bytes=sparse_size(dst,cap,src,n,&sparse_base,&exception_count);
    if (n>=16 && memcmp(src,src+8,8)==0) {
        size_t i=16;
        for (; n-i>=8; i+=8) if (memcmp(src,src+i,8)) break;
        if (n-i<8 && (i==n || memcmp(src,src+i,n-i)==0)) {
            if (sparse_bytes<9) {
                if (cap<sparse_bytes) return 0;
                write_sparse(dst,src,n,sparse_base,exception_count);
                return sparse_bytes;
            }
            if (cap<9) return 0;
            dst[0]=REPEAT64; memcpy(dst+1,src,8); return 9;
        }
    }
    /* Cost-oriented shortcut: sufficiently compact sparse pages skip the
     * lane-extrema pass. Another mode may occasionally be slightly smaller;
     * this fixed policy trades that possibility for less encoding work. */
    if (sparse_bytes<=n/16) {
        if (cap<sparse_bytes) return 0;
        write_sparse(dst,src,n,sparse_base,exception_count);
        return sparse_bytes;
    }
    unsigned mode=RAW;
    size_t best=n+1;
    if (high-low<=15 && 2+n/2+n%2<best) {
        mode=BYTE_DELTA4; best=2+n/2+n%2;
    }
    uint32_t minima[4], maxima[4];
    unsigned widths[4];
    extrema(src,n,minima,maxima);
    for (unsigned lane=0; lane<4; ++lane) widths[lane]=width_for(maxima[lane]-minima[lane]);
    size_t lane_bytes=lanes_size(n,widths);
    if (lane_bytes<best) { best=lane_bytes; mode=LANES32; }
    if (sparse_bytes<best) { best=sparse_bytes; mode=SPARSE32; }
    if (cap<best) return 0;
    dst[0]=(uint8_t)mode;
    if (mode==RAW) memcpy(dst+1,src,n);
    else if (mode==SPARSE32) write_sparse(dst,src,n,sparse_base,exception_count);
    else if (mode==BYTE_DELTA4) {
        dst[1]=low;
        for (size_t i=0; i<n; i+=2) {
            uint8_t a=src[i]-low, b=i+1<n ? src[i+1]-low : 0;
            dst[2+i/2]=(uint8_t)(a | b<<4);
        }
    } else {
        uint8_t *cursor=dst+21;
        for (unsigned lane=0; lane<4; ++lane) {
            dst[1+lane]=(uint8_t)widths[lane];
            store32(dst+5+4*lane,minima[lane]);
            size_t count=lane_count(n/4,lane);
            if (count) pack_lane(cursor,src+4*lane,count,widths[lane],minima[lane]);
            cursor+=packed_size(count,widths[lane]);
        }
        memcpy(cursor,src+(n/4)*4,n%4);
    }
    return best;
}

bool rc_decode(uint8_t *dst, size_t n, const uint8_t *src, size_t m) {
    if ((!dst && n) || !src || !m || !rc_bound(n)) return false;
    switch (src[0]) {
    case RAW:
        if (m!=n+1) return false;
        if (n) memcpy(dst,src+1,n);
        return true;
    case BYTE_UNIFORM:
        if (m!=2 || !n) return false;
        memset(dst,src[1],n); return true;
    case REPEAT64: {
        if (m!=9 || n<8) return false;
        size_t i=0;
        for (; n-i>=8; i+=8) memcpy(dst+i,src+1,8);
        memcpy(dst+i,src+1,n-i);
        return true;
    }
    case BYTE_DELTA4:
        if (m!=2+n/2+n%2 || !n) return false;
        for (size_t i=0; i<n; ++i)
            dst[i]=(uint8_t)(src[1]+((src[2+i/2]>>(4*(i%2)))&15));
        return true;
    case LANES32: {
        if (m<21) return false;
        unsigned widths[4];
        for (unsigned lane=0; lane<4; ++lane) {
            widths[lane]=src[1+lane];
            if (!valid_width(widths[lane])) return false;
        }
        if (lanes_size(n,widths)!=m) return false;
        const uint8_t *cursor=src+21;
        for (unsigned lane=0; lane<4; ++lane) {
            size_t count=lane_count(n/4,lane);
            if (count) unpack_lane(dst+4*lane,cursor,count,widths[lane],load32(src+5+4*lane));
            cursor+=packed_size(count,widths[lane]);
        }
        if (n%4) memcpy(dst+(n/4)*4,cursor,n%4);
        return true;
    }
    case SPARSE32: {
        size_t words=n/4;
        if (m<7 || words<2 || words>65536) return false;
        unsigned count=load16(src+5);
        if (count>words || m!=7+6*(size_t)count+n%4) return false;
        const uint8_t *cursor=src+7;
        unsigned previous=0;
        for (unsigned j=0;j<count;++j) {
            unsigned index=load16(cursor+6*(size_t)j);
            if (index>=words || (j && index<=previous)) return false;
            previous=index;
        }
        uint32_t base=load32(src+1);
        for (size_t i=0;i<words;++i) store32(dst+4*i,base);
        for (unsigned j=0;j<count;++j) {
            unsigned index=load16(cursor);
            store32(dst+4*(size_t)index,load32(cursor+2));
            cursor+=6;
        }
        if (n%4) memcpy(dst+4*words,cursor,n%4);
        return true;
    }
    default: return false;
    }
}
