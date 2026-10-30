/* V2 regression and sparse-mode correctness; no performance measurements. */
#include "codec.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <assert.h>
static uint64_t rng=0x76a5cbff72112345ULL;
static uint32_t rnd(void){rng^=rng>>12;rng^=rng<<25;rng^=rng>>27;return(uint32_t)((rng*0x2545f4914f6cdd1dULL)>>32);}
int main(void){
  unsigned counts[6]={0}; size_t trials=0;
  for(unsigned z=0;z<1800;z++){
    size_t n=z<300?z:(z%4==0?16384+(int)(z%7)-3:rnd()%9000);
    unsigned shape=z%8,offset=z%8;
    uint8_t *a=malloc(n+16),*b=malloc(n+32),*c=malloc(n+16);
    uint8_t *src=a+offset,*enc=b+3,*out=c+5;
    uint8_t pattern[8];for(unsigned j=0;j<8;j++)pattern[j]=(uint8_t)rnd();
    for(size_t i=0;i<n;i++){
      if(shape==0)src[i]=0;
      else if(shape==1)src[i]=pattern[0];
      else if(shape==2)src[i]=pattern[i%8];
      else if(shape==3)src[i]=(uint8_t)(113+(rnd()&15));
      else src[i]=(uint8_t)rnd();
    }
    if(shape>=5){
      uint32_t bases[4]={rnd(),rnd(),rnd(),rnd()};
      unsigned bits=(z%8+1)*4;uint32_t mask=bits==32?UINT32_MAX:((1u<<bits)-1);
      for(size_t i=0;i<n/4;i++){
        uint32_t v=shape==5?bases[i%4]+(rnd()&mask):shape==6?(uint32_t)((int32_t)(rnd()%2048)-1024):bases[i%4]+(rnd()&((i%4==1)?UINT32_MAX:255));
        memcpy(src+4*i,&v,4);
      }
    }
    size_t m=rc_encode(enc,rc_bound(n),src,n);
    assert(m>0&&m<=n+1);assert(enc[0]<6);counts[enc[0]]++;
    assert(rc_decode(out,n,enc,m));assert(!memcmp(out,src,n));
    assert(!rc_decode(out,n,enc,m-1));
    enc[m]=0;assert(!rc_decode(out,n,enc,m+1));
    assert(rc_encode(enc,m-1,src,n)==0);
    uint8_t tag=enc[0];enc[0]=255;assert(!rc_decode(out,n,enc,m));enc[0]=tag;
    free(a);free(b);free(c);trials++;
  }
  /* Sparse bases, exception densities, first-word outliers, large index and
   * the exact eligibility boundary (65536 uint32 words) with every tail. */
  unsigned sparse_cases=0;
  size_t sizes[]={4,128,4096,65535,65536,65537};
  uint32_t bases[]={0,UINT32_C(0x1234fedc),UINT32_C(0xffffffff)};
  for(unsigned si=0;si<sizeof sizes/sizeof sizes[0];++si)
    for(unsigned bi=0;bi<3;++bi)
      for(unsigned tail=0;tail<4;++tail)
        for(unsigned first=0;first<2;++first){
          size_t words=sizes[si],n=4*words+tail;
          uint8_t *src=malloc(n),*enc=malloc(n+1),*out=malloc(n);
          for(size_t i=0;i<words;++i)memcpy(src+4*i,&bases[bi],4);
          for(size_t i=32;i<words;i+=257){uint32_t x=bases[bi]^UINT32_C(0x78aa11bb);memcpy(src+4*i,&x,4);}
          uint32_t x=bases[bi]^UINT32_C(0xcafe7ead);
          memcpy(src+4*(words-1),&x,4);
          if(first)memcpy(src,&x,4);
          for(unsigned i=0;i<tail;++i)src[4*words+i]=(uint8_t)(0x53+i);
          size_t m=rc_encode(enc,n+1,src,n);
          assert(m>0&&m<=n+1&&enc[0]<6);
          assert(rc_decode(out,n,enc,m)&&!memcmp(src,out,n));
          assert(!rc_decode(out,n,enc,m-1));
          if(words>65536)assert(enc[0]!=5);
          if(words>=128&&words<=65536)assert(enc[0]==5);
          assert(rc_encode(enc,m-1,src,n)==0);
          counts[enc[0]]++;sparse_cases++;trials++;
          free(src);free(enc);free(out);
        }
  /* Dense after a uniform prefix: early full-scan abandonment stays lossless. */
  for(unsigned j=0;j<20;++j){
    size_t n=16384+j%4;
    uint8_t *src=calloc(n,1),*enc=malloc(n+1),*out=malloc(n);
    for(size_t i=128;i<n;++i)src[i]=(uint8_t)rnd();
    size_t m=rc_encode(enc,n+1,src,n);assert(m&&m<=n+1);
    assert(rc_decode(out,n,enc,m)&&!memcmp(src,out,n));
    counts[enc[0]]++;trials++;
    free(src);free(enc);free(out);
  }
  /* Explicit malformed sparse frames: count, duplicate, descending and
   * out-of-range exception indices are rejected before materialization. */
  {
    uint8_t src[512],enc[513],saved[513],out[512];
    uint32_t base=UINT32_C(0x1234fedc),value=UINT32_C(0x77889900);
    for(unsigned i=0;i<128;++i)memcpy(src+4*i,&base,4);
    for(unsigned i=0;i<3;++i){unsigned ix=i==0?40:i==1?80:127;memcpy(src+4*ix,&value,4);}
    size_t m=rc_encode(enc,sizeof enc,src,sizeof src);assert(enc[0]==5&&m==25);
    memcpy(saved,enc,m);
    enc[5]=4;assert(!rc_decode(out,sizeof out,enc,m));memcpy(enc,saved,m);
    enc[7]=128;enc[8]=0;assert(!rc_decode(out,sizeof out,enc,m));memcpy(enc,saved,m);
    enc[13]=40;enc[14]=0;assert(!rc_decode(out,sizeof out,enc,m));memcpy(enc,saved,m);
    enc[13]=39;enc[14]=0;assert(!rc_decode(out,sizeof out,enc,m));
    assert(!rc_decode(out,sizeof out,saved,6));
    memcpy(src,&base,4);for(unsigned i=0;i<128;++i)memcpy(src+4*i,&base,4);
    m=rc_encode(enc,sizeof enc,src,sizeof src);assert(enc[0]==5&&m==7);
    assert(rc_decode(out,sizeof out,enc,m)&&!memcmp(src,out,sizeof src));
  }
  assert(rc_bound(SIZE_MAX)==0);
  printf("PASS %zu roundtrips (%u sparse variants) plus malformed sparse checks; modes raw=%u uniform=%u repeat64=%u byte4=%u lanes=%u sparse=%u\n",trials,sparse_cases,counts[0],counts[1],counts[2],counts[3],counts[4],counts[5]);
}
