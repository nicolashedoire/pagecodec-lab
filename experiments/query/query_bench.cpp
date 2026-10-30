// C++17, standalone exact compressed-column query experiment.
// No dependency on or modification to the frozen page codec.
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <numeric>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
#if defined(__aarch64__)
#include <arm_neon.h>
#define QUERY_NEON 1
#endif

namespace {
constexpr size_t BLOCK = 1024, WORDS = BLOCK / 64;
constexpr int32_t MISSING = std::numeric_limits<int32_t>::min();
const std::array<std::string, 6> NAMES = {
    "raw", "for", "bitplane_eager", "bitplane_progressive", "bitplane_adaptive", "rle"};
using Clock = std::chrono::steady_clock;
double ms(Clock::time_point a, Clock::time_point b) {
    return std::chrono::duration<double, std::milli>(b-a).count();
}
struct Answer { uint64_t count=0; int64_t sum=0; };
bool operator==(const Answer& a, const Answer& b) { return a.count==b.count && a.sum==b.sum; }
struct Query { size_t begin, end; int32_t threshold; };
struct Bounds { int32_t minimum=MISSING, maximum=MISSING; };
uint64_t low(unsigned n) { return n==64 ? ~uint64_t(0) : (n==0 ? 0 : (uint64_t(1)<<n)-1); }
unsigned pc(uint64_t x) { return static_cast<unsigned>(__builtin_popcountll(x)); }
unsigned width(uint32_t x) { return x ? 32U-static_cast<unsigned>(__builtin_clz(x)) : 0; }
uint64_t digest(const Answer& a) {
    return a.count*UINT64_C(0x9e3779b97f4a7c15) + static_cast<uint64_t>(a.sum)*UINT64_C(0xbf58476d1ce4e5b9);
}
volatile uint64_t observed;
std::string quote(const std::string& s) {
    std::ostringstream o; o << '"';
    for(unsigned char c:s) {
        if(c=='"'||c=='\\') o << '\\' << c;
        else if(c<32) o << "\\u" << std::hex << std::setw(4) << std::setfill('0') << unsigned(c) << std::dec;
        else o << c;
    }
    return o.str()+'"';
}
std::string basename(const std::string& s) { return s.substr(s.find_last_of("/\\")+1); }
uint64_t hash(const std::vector<int32_t>& v) {
    uint64_t h=UINT64_C(14695981039346656037);
    for(int32_t x:v) for(unsigned j=0;j<4;++j) { h^=(static_cast<uint32_t>(x)>>(8*j))&255; h*=UINT64_C(1099511628211); }
    return h;
}
std::vector<int32_t> read(const std::string& path) {
    std::ifstream f(path, std::ios::binary|std::ios::ate);
    if(!f) throw std::runtime_error("cannot open input: "+path);
    const auto size=f.tellg();
    if(size<=0 || size%4 || size>64*1024*1024) throw std::runtime_error("each input must contain 1..16777216 LE int32 values");
    std::vector<int32_t> out(static_cast<size_t>(size)/4);
    f.seekg(0); f.read(reinterpret_cast<char*>(out.data()), size);
    if(!f) throw std::runtime_error("input read failed");
    uint32_t one=1;
    if(*reinterpret_cast<unsigned char*>(&one)!=1)
        for(auto& x:out) { uint32_t u; std::memcpy(&u,&x,4); u=__builtin_bswap32(u); std::memcpy(&x,&u,4); }
    return out;
}
struct Raw {
    std::vector<int32_t> x,y;
    std::vector<Bounds> bounds;
    void index() {
        bounds.resize((x.size()+BLOCK-1)/BLOCK);
        for(size_t b=0;b<bounds.size();++b) {
            auto& z=bounds[b];
            for(size_t i=b*BLOCK;i<std::min(x.size(),(b+1)*BLOCK);++i) if(x[i]!=MISSING) {
                if(z.minimum==MISSING) z.minimum=z.maximum=x[i];
                else { z.minimum=std::min(z.minimum,x[i]); z.maximum=std::max(z.maximum,x[i]); }
            }
        }
    }
    Answer oracle(const Query& q) const {
        Answer a;
        for(size_t i=q.begin;i<q.end;++i) if(x[i]!=MISSING && x[i]>=q.threshold) {
            ++a.count; if(y[i]!=MISSING) a.sum+=static_cast<int64_t>(y[i]);
        }
        return a;
    }
    Answer query(const Query& q) const {
        Answer a;
        for(size_t b=q.begin/BLOCK;b*BLOCK<q.end;++b) {
            if(bounds[b].maximum==MISSING || bounds[b].maximum<q.threshold) continue;
            size_t i=std::max(q.begin,b*BLOCK), end=std::min(q.end,(b+1)*BLOCK);
#ifdef QUERY_NEON
            uint64x2_t count=vdupq_n_u64(0); int64x2_t sum=vdupq_n_s64(0);
            const int32x4_t missing=vdupq_n_s32(MISSING), threshold=vdupq_n_s32(q.threshold);
            for(;i+4<=end;i+=4) {
                int32x4_t xv=vld1q_s32(x.data()+i), yv=vld1q_s32(y.data()+i);
                uint32x4_t mask=vandq_u32(vmvnq_u32(vceqq_s32(xv,missing)),vcgeq_s32(xv,threshold));
                count=vaddq_u64(count,vpaddlq_u32(vshrq_n_u32(mask,31)));
                uint32x4_t sm=vandq_u32(mask,vmvnq_u32(vceqq_s32(yv,missing)));
                sum=vaddq_s64(sum,vpaddlq_s32(vreinterpretq_s32_u32(vandq_u32(sm,vreinterpretq_u32_s32(yv)))));
            }
            a.count+=vaddvq_u64(count); a.sum+=vaddvq_s64(sum);
#endif
            for(;i<end;++i) {
                const bool match=x[i]!=MISSING && x[i]>=q.threshold;
                a.count+=match; a.sum+=(match && y[i]!=MISSING) ? static_cast<int64_t>(y[i]) : 0;
            }
        }
        return a;
    }
};
struct Meta {
    size_t xoff=0,yoff=0,voff=0;
    int32_t xb=0,yb=0,xmax=MISSING;
    uint16_t rows=0;
    uint8_t wx=0,wy=0;
};
struct Store {
    std::vector<Meta> blocks;
    std::vector<uint64_t> x,y,xvalid,yvalid;
    bool planes;
    explicit Store(const Raw& raw,bool bitplanes):planes(bitplanes) {
        blocks.reserve((raw.x.size()+BLOCK-1)/BLOCK);
        for(size_t start=0;start<raw.x.size();start+=BLOCK) {
            Meta m; m.rows=static_cast<uint16_t>(std::min(BLOCK,raw.x.size()-start));
            bool vx=false,vy=false; int32_t ymax=0;
            for(size_t j=0;j<m.rows;++j) {
                int32_t a=raw.x[start+j],b=raw.y[start+j];
                if(a!=MISSING) { if(!vx) {m.xb=m.xmax=a;vx=true;} else {m.xb=std::min(m.xb,a);m.xmax=std::max(m.xmax,a);} }
                if(b!=MISSING) { if(!vy) {m.yb=ymax=b;vy=true;} else {m.yb=std::min(m.yb,b);ymax=std::max(ymax,b);} }
            }
            m.wx=static_cast<uint8_t>(vx?width(static_cast<uint32_t>(int64_t(m.xmax)-m.xb)):0);
            m.wy=static_cast<uint8_t>(vy?width(static_cast<uint32_t>(int64_t(ymax)-m.yb)):0);
            m.voff=xvalid.size(); xvalid.resize(xvalid.size()+WORDS,0); yvalid.resize(yvalid.size()+WORDS,0);
            m.xoff=x.size();m.yoff=y.size();
            x.resize(x.size()+(planes?m.wx*WORDS:(m.rows*m.wx+63)/64),0);
            y.resize(y.size()+(planes?m.wy*WORDS:(m.rows*m.wy+63)/64),0);
            for(size_t j=0;j<m.rows;++j) {
                if(raw.x[start+j]!=MISSING) {
                    xvalid[m.voff+j/64]|=uint64_t(1)<<(j%64);
                    put(x,m.xoff,m.wx,j,static_cast<uint32_t>(int64_t(raw.x[start+j])-m.xb));
                }
                if(raw.y[start+j]!=MISSING) {
                    yvalid[m.voff+j/64]|=uint64_t(1)<<(j%64);
                    put(y,m.yoff,m.wy,j,static_cast<uint32_t>(int64_t(raw.y[start+j])-m.yb));
                }
            }
            blocks.push_back(m);
        }
    }
    void put(std::vector<uint64_t>& a,size_t off,unsigned w,size_t i,uint32_t v) {
        if(!w) return;
        if(planes) {
            while(v) { unsigned bit=static_cast<unsigned>(__builtin_ctz(v)); a[off+bit*WORDS+i/64]|=uint64_t(1)<<(i%64); v&=v-1; }
        } else {
            size_t pos=i*w, word=off+pos/64; unsigned shift=pos%64;
            a[word]|=uint64_t(v)<<shift;
            if(shift+w>64) a[word+1]|=uint64_t(v)>>(64-shift);
        }
    }
    uint32_t get(const std::vector<uint64_t>& a,size_t off,unsigned w,size_t i) const {
        if(!w) return 0;
        size_t pos=i*w, word=off+pos/64; unsigned shift=pos%64;
        uint64_t v=a[word]>>shift;
        if(shift+w>64) v|=a[word+1]<<(64-shift);
        return static_cast<uint32_t>(v&low(w));
    }
    Answer queryFor(const Query& q) const {
        Answer a;
        for(size_t b=q.begin/BLOCK;b*BLOCK<q.end;++b) {
            const Meta& m=blocks[b]; if(m.xmax==MISSING||m.xmax<q.threshold) continue;
            size_t first=std::max(q.begin,b*BLOCK)-b*BLOCK,last=std::min(q.end,b*BLOCK+m.rows)-b*BLOCK;
            const bool all=m.xb>=q.threshold;
            for(size_t j=first;j<last;++j) {
                uint64_t bit=uint64_t(1)<<(j%64);
                if((xvalid[m.voff+j/64]&bit) && (all || int64_t(m.xb)+get(x,m.xoff,m.wx,j)>=q.threshold)) {
                    ++a.count;
                    if(yvalid[m.voff+j/64]&bit) a.sum+=int64_t(m.yb)+get(y,m.yoff,m.wy,j);
                }
            }
        }
        return a;
    }
    uint64_t predicate(const Meta& m,size_t word,uint64_t valid,int32_t threshold,bool progressive) const {
        if(threshold<=m.xb) return valid;
        const uint32_t t=static_cast<uint32_t>(int64_t(threshold)-m.xb);
        uint64_t equal=valid,greater=0;
        for(unsigned k=m.wx;k>0;--k) {
            unsigned j=k-1; uint64_t ones=x[m.xoff+j*WORDS+word];
            if((t>>j)&1) equal&=ones;
            else {greater|=equal&ones; equal&=~ones;}
            if(progressive) {
                if(!equal) return greater;
                if(!(t&low(j))) return greater|equal;
            }
        }
        return greater|equal;
    }
    Answer queryPlanes(const Query& q,int mode,const std::vector<std::array<uint16_t,16>>& model,
                       std::array<uint64_t,WORDS>& scratch) const {
        Answer a;
        for(size_t b=q.begin/BLOCK;b*BLOCK<q.end;++b) {
            const Meta& m=blocks[b]; if(m.xmax==MISSING||m.xmax<q.threshold) continue;
            size_t first=std::max(q.begin,b*BLOCK)-b*BLOCK,last=std::min(q.end,b*BLOCK+m.rows)-b*BLOCK;
            bool progressive=mode==3;
            if(mode==4 && q.threshold>m.xb) {
                uint32_t t=static_cast<uint32_t>(int64_t(q.threshold)-m.xb);
                unsigned shift=m.wx>4?m.wx-4:0;
                progressive=model[b][t>>shift]<=m.rows/8;
            }
            for(size_t w=first/64;w<(last+63)/64;++w) {
                unsigned lo=static_cast<unsigned>(std::max(first,w*64)-w*64);
                unsigned hi=static_cast<unsigned>(std::min(last,(w+1)*64)-w*64);
                uint64_t valid=xvalid[m.voff+w] & low(hi) & ~low(lo);
                uint64_t mask=predicate(m,w,valid,q.threshold,progressive);
                a.count+=pc(mask); scratch[w]=mask&yvalid[m.voff+w];
                a.sum+=int64_t(m.yb)*pc(scratch[w]);
            }
            for(unsigned bit=0;bit<m.wy;++bit) {
                unsigned cnt=0;
                for(size_t w=first/64;w<(last+63)/64;++w) cnt+=pc(scratch[w]&y[m.yoff+bit*WORDS+w]);
                a.sum+=int64_t(uint64_t(1)<<bit)*cnt;
            }
        }
        return a;
    }
    size_t payload() const {return 8*(x.size()+y.size()+xvalid.size()+yvalid.size());}
    size_t capacity() const {return sizeof(*this)+sizeof(Meta)*blocks.capacity()+8*(x.capacity()+y.capacity()+xvalid.capacity()+yvalid.capacity());}
    void validateRows(const Raw& raw) const {
        for(size_t b=0;b<blocks.size();++b) {
            const auto& m=blocks[b];
            for(size_t j=0;j<m.rows;++j) for(unsigned column=0;column<2;++column) {
                const auto& data=column?y:x;const auto& valid=column?yvalid:xvalid;
                size_t off=column?m.yoff:m.xoff;unsigned w=column?m.wy:m.wx;
                int64_t value=MISSING;
                if((valid[m.voff+j/64]>>(j%64))&1) {
                    uint32_t residual=0;
                    if(planes) for(unsigned bit=0;bit<w;++bit) residual|=static_cast<uint32_t>((data[off+bit*WORDS+j/64]>>(j%64))&1)<<bit;
                    else residual=get(data,off,w,j);
                    value=int64_t(column?m.yb:m.xb)+residual;
                }
                if(value!=(column?raw.y:raw.x)[b*BLOCK+j]) throw std::runtime_error("representation round-trip mismatch");
            }
        }
    }
};
struct Run {uint32_t start,length;int32_t x,y;};
struct Rle {
    std::vector<Run> runs;
    explicit Rle(const Raw& raw) {
        for(size_t i=0;i<raw.x.size();++i) {
            if(!runs.empty()&&runs.back().x==raw.x[i]&&runs.back().y==raw.y[i]) ++runs.back().length;
            else runs.push_back({static_cast<uint32_t>(i),1,raw.x[i],raw.y[i]});
        }
    }
    Answer query(const Query& q) const {
        Answer a; if(q.begin==q.end) return a;
        auto it=std::upper_bound(runs.begin(),runs.end(),q.begin,[](size_t index,const Run& r){return index<r.start;});
        if(it!=runs.begin()) --it;
        for(;it!=runs.end()&&it->start<q.end;++it) {
            size_t first=std::max(q.begin,size_t(it->start)),last=std::min(q.end,size_t(it->start)+it->length);
            if(first<last&&it->x!=MISSING&&it->x>=q.threshold) {
                a.count+=last-first; if(it->y!=MISSING) a.sum+=int64_t(it->y)*static_cast<int64_t>(last-first);
            }
        }
        return a;
    }
};
struct Footprint {size_t payload=0,metadata=0,capacity=0,scratch=0,model=0; double construction=0;};
struct Workload {std::string name;int32_t threshold;std::vector<Query> queries;std::vector<Answer> answers;size_t logicalRows=0;unsigned inner=1;};
struct Runner {
    Raw raw; std::unique_ptr<Store> packed,planes;std::unique_ptr<Rle> rle;
    std::vector<std::array<uint16_t,16>> model;
    std::array<uint64_t,WORDS> scratch{};
    Answer query(int method,const Query& q) {
        if(q.begin==q.end) return {};
        if(method==0) return raw.query(q);
        if(method==1) return packed->queryFor(q);
        if(method==5) return rle->query(q);
        return planes->queryPlanes(q,method,model,scratch);
    }
};
int64_t number(const std::string& s) {size_t pos=0;auto n=std::stoll(s,&pos);if(pos!=s.size())throw std::runtime_error("invalid number");return n;}
int32_t int32(const std::string& s) {auto v=number(s);if(v<INT32_MIN||v>INT32_MAX)throw std::runtime_error("threshold outside int32");return static_cast<int32_t>(v);}
double median(std::vector<double> v) {std::sort(v.begin(),v.end());size_t n=v.size();return n%2?v[n/2]:(v[n/2-1]+v[n/2])/2;}
}

int main(int argc,char** argv) try {
    std::string xpath,ypath,output,thresholdArg,only;unsigned repetitions=9;uint64_t seed=20261030;
    bool validateOnly=false;
    for(int i=1;i<argc;++i) {
        std::string option=argv[i];
        if(option=="--help") {std::cout<<"query_bench --x FILE --y FILE [--thresholds INT,INT,INT] [--repetitions 9] [--seed 20261030] [--only-method NAME] [--validate-only] [--output FILE]\n";return 0;}
        if(option=="--validate-only") {validateOnly=true;continue;}
        if(i+1==argc) throw std::runtime_error("missing option value");
        std::string value=argv[++i];
        if(option=="--x") xpath=value;else if(option=="--y") ypath=value;
        else if(option=="--output") output=value;else if(option=="--thresholds") thresholdArg=value;
        else if(option=="--only-method") only=value;
        else if(option=="--repetitions") {auto v=number(value);if(v<1||v>31)throw std::runtime_error("repetitions must be 1..31");repetitions=static_cast<unsigned>(v);}
        else if(option=="--seed") {auto v=number(value);if(v<0)throw std::runtime_error("seed must be nonnegative");seed=static_cast<uint64_t>(v);}
        else throw std::runtime_error("unknown option: "+option);
    }
    if(xpath.empty()||ypath.empty()) throw std::runtime_error("--x and --y required");
    std::vector<int> methods;for(int m=0;m<6;++m) if(only.empty()||only==NAMES[m])methods.push_back(m);
    if(methods.empty())throw std::runtime_error("unknown method");
    auto has=[&](int m){return std::find(methods.begin(),methods.end(),m)!=methods.end();};
    Runner runner;runner.raw.x=read(xpath);runner.raw.y=read(ypath);
    if(runner.raw.x.size()!=runner.raw.y.size())throw std::runtime_error("input row counts differ");
    size_t rows=runner.raw.x.size();uint64_t xhash=hash(runner.raw.x),yhash=hash(runner.raw.y);
    std::vector<int32_t> thresholds;
    if(!thresholdArg.empty()) {std::stringstream s(thresholdArg);std::string token;while(std::getline(s,token,','))thresholds.push_back(int32(token));}
    else {
        std::vector<int32_t> sample;size_t calibration=(rows+4)/5;
        for(size_t i=0;i<calibration;++i)if(runner.raw.x[i]!=MISSING)sample.push_back(runner.raw.x[i]);
        if(sample.empty())thresholds={0,0,0};else {std::sort(sample.begin(),sample.end());for(unsigned p:{10,50,90})thresholds.push_back(sample[(sample.size()-1)*p/100]);}
    }
    if(thresholds.empty()||thresholds.size()>16)throw std::runtime_error("provide 1..16 thresholds");
    std::array<Footprint,6> footprints{};
    auto start=Clock::now();if(has(0))runner.raw.index();double rawBuild=ms(start,Clock::now());
    footprints[0]={rows*8,runner.raw.bounds.size()*sizeof(Bounds),sizeof(Raw)+4*(runner.raw.x.capacity()+runner.raw.y.capacity())+sizeof(Bounds)*runner.raw.bounds.capacity(),0,0,rawBuild};
    if(has(1)) {
        start=Clock::now();runner.packed=std::make_unique<Store>(runner.raw,false);double elapsed=ms(start,Clock::now());
        footprints[1]={runner.packed->payload(),runner.packed->blocks.size()*sizeof(Meta),runner.packed->capacity(),0,0,elapsed};
    }
    if(has(2)||has(3)||has(4)) {
        start=Clock::now();runner.planes=std::make_unique<Store>(runner.raw,true);double elapsed=ms(start,Clock::now());
        for(int m:{2,3,4}) footprints[m]={runner.planes->payload(),runner.planes->blocks.size()*sizeof(Meta),runner.planes->capacity(),sizeof(runner.scratch),0,elapsed};
        if(has(4)) {
            start=Clock::now();runner.model.resize(runner.planes->blocks.size());
            for(size_t b=0;b<runner.model.size();++b) {
                const auto& z=runner.planes->blocks[b];unsigned shift=z.wx>4?z.wx-4:0;
                for(size_t j=0;j<z.rows;++j) if(runner.raw.x[b*BLOCK+j]!=MISSING) {
                    auto residual=static_cast<uint32_t>(int64_t(runner.raw.x[b*BLOCK+j])-z.xb);
                    ++runner.model[b][residual>>shift];
                }
            }
            footprints[4].model=runner.model.size()*sizeof(runner.model[0]);
            footprints[4].metadata+=footprints[4].model;
            footprints[4].capacity+=sizeof(runner.model)+runner.model.capacity()*sizeof(runner.model[0]);
            footprints[4].construction+=ms(start,Clock::now());
        }
    }
    if(has(5)) {
        start=Clock::now();runner.rle=std::make_unique<Rle>(runner.raw);double elapsed=ms(start,Clock::now());
        footprints[5]={runner.rle->runs.size()*8,runner.rle->runs.size()*8,sizeof(Rle)+runner.rle->runs.capacity()*sizeof(Run),0,0,elapsed};
    }
    if(runner.packed)runner.packed->validateRows(runner.raw);
    if(runner.planes)runner.planes->validateRows(runner.raw);
    if(runner.rle) for(const auto& run:runner.rle->runs) for(size_t j=run.start;j<size_t(run.start)+run.length;++j)
        if(runner.raw.x[j]!=run.x||runner.raw.y[j]!=run.y)throw std::runtime_error("RLE round-trip mismatch");
    std::mt19937_64 rng(seed);std::vector<Workload> workloads;
    for(int32_t threshold:thresholds) for(unsigned scale=0;scale<3;++scale) {
        size_t length=scale==0?rows:std::min(rows,scale==1?size_t(65536):size_t(1024));
        unsigned count=scale==0?1:scale==1?4:16;
        Workload w;w.name=scale==0?"full":scale==1?"medium_64k":"small_1k";w.threshold=threshold;
        for(unsigned i=0;i<count;++i) {
            size_t begin=scale==0?0:static_cast<size_t>(rng()%(rows-length+1));
            Query q{begin,begin+length,threshold};w.queries.push_back(q);w.answers.push_back(runner.raw.oracle(q));w.logicalRows+=length;
        }
        w.inner=static_cast<unsigned>(std::min(size_t(64),std::max(size_t(1),(1024*1024+w.logicalRows*8-1)/(w.logicalRows*8))));
        workloads.push_back(std::move(w));
    }
    std::vector<Query> checks;
    for(int32_t t:{INT32_MIN,INT32_MIN+1,-1,0,1,INT32_MAX}) {
        checks.push_back({0,rows,t});checks.push_back({0,0,t});checks.push_back({rows,rows,t});
        checks.push_back({rows-1,rows,t});
        if(rows>63)checks.push_back({63,std::min(rows,size_t(1025)),t});
        if(rows>1023)checks.push_back({1023,std::min(rows,size_t(2049)),t});
    }
    for(unsigned i=0;i<64;++i) {
        size_t begin=static_cast<size_t>(rng()%rows),length=static_cast<size_t>(rng()%(std::min(rows-begin,size_t(65536))+1));
        uint32_t bits=static_cast<uint32_t>(rng());int32_t threshold;std::memcpy(&threshold,&bits,4);
        checks.push_back({begin,begin+length,threshold});
    }
    size_t validated=0;
    auto validate=[&](const Query& q,const Answer& expected){for(int method:methods){if(!(runner.query(method,q)==expected))throw std::runtime_error("exact query mismatch: "+NAMES[method]);++validated;}};
    for(const auto& q:checks)validate(q,runner.raw.oracle(q));
    for(const auto& w:workloads)for(size_t i=0;i<w.queries.size();++i)validate(w.queries[i],w.answers[i]);
    // Isolated non-raw probes keep the expected answers, then release input arrays.
    bool released=!only.empty()&&!has(0);
    if(released) {std::vector<int32_t>().swap(runner.raw.x);std::vector<int32_t>().swap(runner.raw.y);std::vector<Bounds>().swap(runner.raw.bounds);}
    struct Measurement {std::array<std::vector<double>,6> times;std::vector<std::vector<int>> order;};
    std::vector<Measurement> measurements(workloads.size());
    if(!validateOnly)for(size_t wi=0;wi<workloads.size();++wi) {
        const auto& w=workloads[wi];uint64_t expected=0;for(const auto& a:w.answers)expected+=digest(a);
        auto trial=[&](int method) {
            uint64_t value=0;auto a=Clock::now();
            for(unsigned repeat=0;repeat<w.inner;++repeat)for(const auto& q:w.queries)value+=digest(runner.query(method,q));
            auto b=Clock::now();observed=value;
            if(value!=expected*w.inner)throw std::runtime_error("timed query checksum mismatch");
            return ms(a,b)/w.inner;
        };
        for(int method:methods)trial(method);
        for(unsigned r=0;r<repetitions;++r) {
            auto order=methods;
            for(size_t i=order.size();i>1;--i)std::swap(order[i-1],order[static_cast<size_t>(rng()%i)]);
            measurements[wi].order.push_back(order);
            for(int method:order)measurements[wi].times[method].push_back(trial(method));
        }
    }
    std::ostringstream out;out<<std::setprecision(10);
    out<<"{\"schema_version\":1,\"experiment\":\"exact_masked_queries\",\"rows\":"<<rows<<",\"block_rows\":"<<BLOCK
       <<",\"x_name\":"<<quote(basename(xpath))<<",\"y_name\":"<<quote(basename(ypath))<<",\"x_fnv1a64\":"<<quote(std::to_string(xhash))
       <<",\"y_fnv1a64\":"<<quote(std::to_string(yhash))<<",\"missing_sentinel\":"<<MISSING<<",\"seed\":"<<seed
       <<",\"repetitions\":"<<repetitions<<",\"warmup_rounds_per_workload_method\":1,\"validate_only\":"<<(validateOnly?"true":"false")
       <<",\"validation_cases\":"<<validated<<",\"verified\":true,\"representation_roundtrips_verified\":true,\"input_arrays_released_before_timing\":"<<(released?"true":"false")
       <<",\"input_arrays_retained_capacity_bytes\":"<<4*(runner.raw.x.capacity()+runner.raw.y.capacity())
       <<",\"threshold_source\":"<<quote(thresholdArg.empty()?"p10,p50,p90 from valid x in first ceil(rows/5) rows; all rows remain queryable":"explicit CLI thresholds")
       <<",\"semantics\":\"count valid x >= threshold in [begin,end); sum corresponding nonmissing y; empty sum=0; exact int64 accumulation\""
       <<",\"timing_scope\":\"single-thread warm repeated query schedules; no full output materialization; allocation, construction, IO and oracle validation excluded; answer digest included; per-schedule times divide batched duration by inner_iterations\""
       <<",\"memory_scope\":\"conceptual per-method owned representation capacities plus explicit reusable scratch, not RSS; vector capacity and object headers counted, allocator metadata/compiler stack excluded; all-method run retains raw oracle and multiple stores; eager/progressive/adaptive share one physical bitplane store\""
#ifdef QUERY_NEON
       <<",\"raw_kernel\":\"ARM NEON 4 int32 lanes with int64 widening accumulation\""
#else
       <<",\"raw_kernel\":\"portable compiler-optimizable scalar loop\""
#endif
       <<",\"methods\":[";
    for(size_t i=0;i<methods.size();++i) {
        int m=methods[i];const auto& f=footprints[m];if(i)out<<',';
        out<<"{\"method\":"<<quote(NAMES[m])<<",\"construction_ms\":"<<f.construction<<",\"payload_bytes\":"<<f.payload
           <<",\"metadata_bytes\":"<<f.metadata<<",\"representation_bytes\":"<<f.payload+f.metadata
           <<",\"allocated_capacity_bytes\":"<<f.capacity<<",\"explicit_scratch_bytes\":"<<f.scratch
           <<",\"owned_working_bytes\":"<<f.capacity+f.scratch<<",\"model_metadata_bytes\":"<<f.model<<'}';
    }
    out<<"],\"workloads\":[";
    for(size_t wi=0;wi<workloads.size();++wi) {
        if(wi)out<<',';const auto& w=workloads[wi];const auto& measurement=measurements[wi];
        out<<"{\"kind\":"<<quote(w.name)<<",\"threshold\":"<<w.threshold<<",\"logical_rows_per_schedule\":"<<w.logicalRows
           <<",\"inner_iterations\":"<<w.inner<<",\"queries\":[";
        for(size_t j=0;j<w.queries.size();++j) {
            if(j)out<<',';out<<"{\"begin\":"<<w.queries[j].begin<<",\"end\":"<<w.queries[j].end<<",\"count\":"<<w.answers[j].count<<",\"sum\":"<<w.answers[j].sum<<'}';
        }
        out<<"],\"trial_order\":[";
        for(size_t j=0;j<measurement.order.size();++j) {if(j)out<<',';out<<'[';for(size_t k=0;k<measurement.order[j].size();++k){if(k)out<<',';out<<quote(NAMES[measurement.order[j][k]]);}out<<']';}
        out<<"],\"results\":[";
        for(size_t j=0;j<methods.size();++j) {
            if(j)out<<',';int method=methods[j];const auto& times=measurement.times[method];
            out<<"{\"method\":"<<quote(NAMES[method])<<",\"verified\":true,\"median_ms\":"<<(times.empty()?0:median(times))<<",\"samples_ms\":[";
            for(size_t k=0;k<times.size();++k){if(k)out<<',';out<<times[k];}out<<"]}";
        }
        out<<"]}";
    }
    out<<"]}\n";
    if(output.empty())std::cout<<out.str();else {std::ofstream f(output);if(!f)throw std::runtime_error("cannot open output");f<<out.str();if(!f)throw std::runtime_error("output write failed");}
    return 0;
} catch(const std::exception& e) {std::cerr<<"query_bench: "<<e.what()<<'\n';return 1;}
