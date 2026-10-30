// Exact offline analytical report, including text input and representation construction.
// Intentionally independent of the previously frozen microbenchmark sources.
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>
#if defined(__aarch64__)
#include <arm_neon.h>
#define REPORT_NEON 1
#endif
#if defined(__unix__) || defined(__APPLE__)
#include <sys/resource.h>
#endif

namespace {
constexpr size_t BLOCK=1024, WORDS=16, MAX_ROWS=16777216;
constexpr int32_t MISSING=std::numeric_limits<int32_t>::min();
using Clock=std::chrono::steady_clock;
double elapsed(Clock::time_point a,Clock::time_point b) {return std::chrono::duration<double,std::milli>(b-a).count();}
struct Answer {uint64_t count=0;int64_t sum=0;};
struct Query {size_t begin=0,end=0;int32_t threshold=0;};
struct Options {std::string input,format,method,queries,output;size_t rows=0,skip=0;};
const std::array<std::string,5> METHODS={"raw","bitplane","bitplane_adaptive","sorted_blocks","stream_sweep"};
uint64_t low(unsigned n) {return n==64?~uint64_t(0):(n?(uint64_t(1)<<n)-1:0);}
unsigned width(uint32_t x) {return x?32U-static_cast<unsigned>(__builtin_clz(x)):0;}
unsigned popcount(uint64_t x) {return static_cast<unsigned>(__builtin_popcountll(x));}
std::string quote(const std::string& value) {
    std::ostringstream out;out<<'"';
    for(unsigned char c:value) {
        if(c=='"'||c=='\\')out<<'\\'<<c;
        else if(c<32)out<<"\\u"<<std::hex<<std::setw(4)<<std::setfill('0')<<unsigned(c)<<std::dec;
        else out<<c;
    }
    out<<'"';return out.str();
}
std::string basename(const std::string& p) {return p.substr(p.find_last_of("/\\")+1);}
size_t positive(const std::string& value,bool zero=false) {
    if(value.empty()||value[0]=='-')throw std::runtime_error("Invalid unsigned integer");
    size_t used=0;const auto n=std::stoull(value,&used);
    if(used!=value.size()||n>MAX_ROWS||(!zero&&!n))throw std::runtime_error("Integer outside supported limits");
    return static_cast<size_t>(n);
}
Options parse_options(int argc,char** argv) {
    Options o;
    for(int i=1;i<argc;++i) {
        const std::string key=argv[i];
        if(key=="--help") {std::cout<<"report-bench --input FILE --format household|covertype|bike --method raw|bitplane|bitplane_adaptive|sorted_blocks|stream_sweep --rows N --skip-rows N --queries TSV --output JSON\n";std::exit(0);}
        if(i+1==argc)throw std::runtime_error("Missing argument value");
        const std::string value=argv[++i];
        if(key=="--input")o.input=value;else if(key=="--format")o.format=value;
        else if(key=="--method")o.method=value;else if(key=="--queries")o.queries=value;
        else if(key=="--output")o.output=value;else if(key=="--rows")o.rows=positive(value);
        else if(key=="--skip-rows")o.skip=positive(value,true);else throw std::runtime_error("Unknown option");
    }
    if(o.input.empty()||o.queries.empty()||o.output.empty()||!o.rows)throw std::runtime_error("Required option missing");
    if(o.format!="household"&&o.format!="covertype"&&o.format!="bike")throw std::runtime_error("Unknown source format");
    if(std::find(METHODS.begin(),METHODS.end(),o.method)==METHODS.end())throw std::runtime_error("Unknown method");
    return o;
}
int32_t integer(std::string_view value,bool scaled=false) {
    if(value.empty()||value=="?")return MISSING;
    size_t i=0;bool negative=false;
    if(value[i]=='-'||value[i]=='+') {negative=value[i]=='-';if(++i==value.size())throw std::runtime_error("Invalid integer");}
    uint64_t whole=0,fraction=0;unsigned digits=0;bool any=false;
    for(;i<value.size()&&value[i]!='.';++i) {
        const char c=value[i];if(c<'0'||c>'9')throw std::runtime_error("Invalid numeric field");
        any=true;whole=whole*10+unsigned(c-'0');if(whole>UINT32_MAX)throw std::runtime_error("Numeric field overflow");
    }
    if(!any)throw std::runtime_error("Invalid numeric field");
    if(i<value.size()) {
        if(!scaled)throw std::runtime_error("Unexpected fractional integer");
        ++i;
        for(;i<value.size();++i) {
            const char c=value[i];if(c<'0'||c>'9')throw std::runtime_error("Invalid decimal");
            if(digits<3) {fraction=fraction*10+unsigned(c-'0');++digits;}
            else if(c!='0')throw std::runtime_error("Decimal cannot be represented exactly at scale 1000");
        }
    }
    if(scaled) {while(digits<3){fraction*=10;++digits;}whole=whole*1000+fraction;}
    const int64_t result=negative?-static_cast<int64_t>(whole):static_cast<int64_t>(whole);
    if(result<INT32_MIN||result>INT32_MAX)throw std::runtime_error("Value outside int32");
    return static_cast<int32_t>(result);
}
std::vector<Query> read_queries(const Options& o) {
    std::ifstream input(o.queries);if(!input)throw std::runtime_error("Cannot open query plan");
    std::vector<Query> result;std::string line;
    while(std::getline(input,line)) {
        if(!line.empty()&&line.back()=='\r')line.pop_back();
        const auto a=line.find('\t'),b=a==std::string::npos?a:line.find('\t',a+1);
        if(a==std::string::npos||b==std::string::npos||line.find('\t',b+1)!=std::string::npos)throw std::runtime_error("Query plan requires begin TAB end TAB threshold");
        const auto threshold=std::string_view(line).substr(b+1);
        if(threshold.empty()||threshold=="?")throw std::runtime_error("Query threshold must be an integer");
        Query q{positive(line.substr(0,a),true),positive(line.substr(a+1,b-a-1),true),integer(threshold)};
        if(q.begin>q.end||q.end>o.rows)throw std::runtime_error("Query window outside evaluation rows");
        result.push_back(q);if(result.size()>65536)throw std::runtime_error("Query plan exceeds 65536 requests");
    }
    if(input.bad()||result.empty())throw std::runtime_error("Empty or unreadable query plan");
    return result;
}
struct Bounds {int32_t minimum=MISSING,maximum=MISSING;uint32_t valid=0;int64_t sum=0;};
Bounds bounds(const int32_t* x,const int32_t* y,size_t n) {
    Bounds z;
    for(size_t i=0;i<n;++i)if(x[i]!=MISSING) {
        if(!z.valid)z.minimum=z.maximum=x[i];else {z.minimum=std::min(z.minimum,x[i]);z.maximum=std::max(z.maximum,x[i]);}
        ++z.valid;if(y[i]!=MISSING)z.sum+=int64_t(y[i]);
    }
    return z;
}
Answer scan(const int32_t* x,const int32_t* y,size_t n,int32_t threshold) {
    Answer result;size_t i=0;
#ifdef REPORT_NEON
    uint64x2_t count=vdupq_n_u64(0);int64x2_t sum=vdupq_n_s64(0);
    const int32x4_t missing=vdupq_n_s32(MISSING),limit=vdupq_n_s32(threshold);
    for(;i+4<=n;i+=4) {
        const int32x4_t xv=vld1q_s32(x+i),yv=vld1q_s32(y+i);
        const uint32x4_t match=vandq_u32(vmvnq_u32(vceqq_s32(xv,missing)),vcgeq_s32(xv,limit));
        count=vaddq_u64(count,vpaddlq_u32(vshrq_n_u32(match,31)));
        const auto valid=vandq_u32(match,vmvnq_u32(vceqq_s32(yv,missing)));
        sum=vaddq_s64(sum,vpaddlq_s32(vreinterpretq_s32_u32(vandq_u32(valid,vreinterpretq_u32_s32(yv)))));
    }
    result.count=vaddvq_u64(count);result.sum=vaddvq_s64(sum);
#endif
    for(;i<n;++i)if(x[i]!=MISSING&&x[i]>=threshold){++result.count;if(y[i]!=MISSING)result.sum+=int64_t(y[i]);}
    return result;
}
struct PlaneBlock {
    Bounds bounds;int32_t ybase=0;uint16_t rows=0;uint8_t wx=0,wy=0;
    size_t words=0;std::unique_ptr<uint64_t[]> data;
};
struct SortedBlock {size_t valid=0;std::unique_ptr<int32_t[]> keys;std::unique_ptr<int64_t[]> prefix;};
struct Event {size_t position=0,query=0;bool end=false;};
struct Store {
    const std::string& method;size_t n=0,expected=0;
    std::vector<int32_t> x,y;
    std::vector<Bounds> index;
    std::vector<PlaneBlock> planes;
    std::vector<SortedBlock> sorted;
    std::vector<int32_t> thresholds;
    std::vector<Event> events;
    std::vector<uint64_t> tree_count;
    std::vector<int64_t> tree_sum;
    size_t next_event=0;uint64_t sweep_count=0;int64_t sweep_sum=0;
    std::vector<Answer>& answers;
    Store(const Options& o,const std::vector<Query>& queries,std::vector<Answer>& results):method(o.method),expected(o.rows),answers(results) {
        const size_t blocks=(expected+BLOCK-1)/BLOCK;
        if(method=="raw"||method=="sorted_blocks") {x.reserve(expected);y.reserve(expected);index.reserve(blocks);}
        if(method=="sorted_blocks")sorted.reserve(blocks);
        if(method=="bitplane"||method=="bitplane_adaptive")planes.reserve(blocks);
        if(method=="stream_sweep") {
            thresholds.reserve(queries.size());events.reserve(2*queries.size());
            for(size_t i=0;i<queries.size();++i) {thresholds.push_back(queries[i].threshold);events.push_back({queries[i].begin,i,false});events.push_back({queries[i].end,i,true});}
            std::sort(thresholds.begin(),thresholds.end());thresholds.erase(std::unique(thresholds.begin(),thresholds.end()),thresholds.end());
            std::sort(events.begin(),events.end(),[](const Event& a,const Event& b){return a.position<b.position;});
            tree_count.assign(thresholds.size()+1,0);tree_sum.assign(thresholds.size()+1,0);
        }
    }
    void event_prefix(size_t position,const std::vector<Query>& queries) {
        while(next_event<events.size()&&events[next_event].position==position) {
            const auto& e=events[next_event++];const auto& q=queries[e.query];
            size_t i=static_cast<size_t>(std::lower_bound(thresholds.begin(),thresholds.end(),q.threshold)-thresholds.begin());
            uint64_t below_count=0;int64_t below_sum=0;
            for(;i;i-=i&(~i+1)){below_count+=tree_count[i];below_sum+=tree_sum[i];}
            const uint64_t count=sweep_count-below_count;const int64_t sum=sweep_sum-below_sum;
            if(e.end){answers[e.query].count+=count;answers[e.query].sum+=sum;}
            else {answers[e.query].count-=count;answers[e.query].sum-=sum;}
        }
    }
    void sweep_row(int32_t a,int32_t b,const std::vector<Query>& queries) {
        event_prefix(n,queries);
        if(a!=MISSING) {
            size_t rank=static_cast<size_t>(std::upper_bound(thresholds.begin(),thresholds.end(),a)-thresholds.begin());
            if(rank) {
                ++sweep_count;const int64_t value=b==MISSING?0:int64_t(b);sweep_sum+=value;
                for(size_t i=rank;i<tree_count.size();i+=i&(~i+1)){++tree_count[i];tree_sum[i]+=value;}
            }
        }
        ++n;
    }
    void add(const int32_t* a,const int32_t* b,size_t count) {
        const Bounds z=bounds(a,b,count);
        if(method=="raw"||method=="sorted_blocks") {x.insert(x.end(),a,a+count);y.insert(y.end(),b,b+count);index.push_back(z);}
        if(method=="sorted_blocks") {
            std::array<std::pair<int32_t,int32_t>,BLOCK> pairs{};size_t valid=0;
            for(size_t i=0;i<count;++i)if(a[i]!=MISSING)pairs[valid++]={a[i],b[i]};
            std::sort(pairs.begin(),pairs.begin()+static_cast<std::ptrdiff_t>(valid));
            SortedBlock s;s.valid=valid;s.keys=std::make_unique<int32_t[]>(valid);s.prefix=std::make_unique<int64_t[]>(valid+1);
            for(size_t i=0;i<valid;++i){s.keys[i]=pairs[i].first;s.prefix[i+1]=s.prefix[i]+(pairs[i].second==MISSING?0:int64_t(pairs[i].second));}
            sorted.push_back(std::move(s));
        }
        if(method=="bitplane"||method=="bitplane_adaptive") {
            PlaneBlock p;p.bounds=z;p.rows=static_cast<uint16_t>(count);
            bool yvalid=false;int32_t ymax=0;
            for(size_t i=0;i<count;++i)if(b[i]!=MISSING){if(!yvalid){p.ybase=ymax=b[i];yvalid=true;}else {p.ybase=std::min(p.ybase,b[i]);ymax=std::max(ymax,b[i]);}}
            p.wx=static_cast<uint8_t>(z.valid?width(static_cast<uint32_t>(int64_t(z.maximum)-z.minimum)):0);
            p.wy=static_cast<uint8_t>(yvalid?width(static_cast<uint32_t>(int64_t(ymax)-p.ybase)):0);
            const bool adaptive=method=="bitplane_adaptive";
            p.words=(2+p.wx+p.wy)*WORDS+(adaptive?4:0);p.data=std::make_unique<uint64_t[]>(p.words);
            std::array<uint16_t,16> histogram{};
            for(size_t i=0;i<count;++i) {
                const uint64_t bit=uint64_t(1)<<(i%64);const size_t word=i/64;
                if(a[i]!=MISSING) {
                    p.data[word]|=bit;uint32_t v=static_cast<uint32_t>(int64_t(a[i])-z.minimum);
                    if(adaptive)++histogram[v>>(p.wx>4?p.wx-4:0)];
                    while(v){const unsigned k=static_cast<unsigned>(__builtin_ctz(v));p.data[2*WORDS+k*WORDS+word]|=bit;v&=v-1;}
                }
                if(b[i]!=MISSING) {
                    p.data[WORDS+word]|=bit;uint32_t v=static_cast<uint32_t>(int64_t(b[i])-p.ybase);
                    while(v){const unsigned k=static_cast<unsigned>(__builtin_ctz(v));p.data[(2+p.wx)*WORDS+k*WORDS+word]|=bit;v&=v-1;}
                }
            }
            if(adaptive)for(size_t k=0;k<16;++k)p.data[(2+p.wx+p.wy)*WORDS+k/4]|=uint64_t(histogram[k])<<(16*(k%4));
            planes.push_back(std::move(p));
        }
        n+=count;
    }
    Answer query(const Query& q) const {
        Answer result;if(q.begin==q.end)return result;
        for(size_t block=q.begin/BLOCK;block*BLOCK<q.end;++block) {
            const size_t start=block*BLOCK,end=std::min(n,start+BLOCK),first=std::max(q.begin,start),last=std::min(q.end,end);
            const bool is_plane=method=="bitplane"||method=="bitplane_adaptive";
            const Bounds& z=is_plane?planes[block].bounds:index[block];
            if(!z.valid||z.maximum<q.threshold)continue;
            if(first==start&&last==end&&z.minimum>=q.threshold) {result.count+=z.valid;result.sum+=z.sum;continue;}
            if(!is_plane) {
                if(method=="sorted_blocks"&&first==start&&last==end) {
                    const auto& s=sorted[block];const size_t at=static_cast<size_t>(std::lower_bound(s.keys.get(),s.keys.get()+s.valid,q.threshold)-s.keys.get());
                    result.count+=s.valid-at;result.sum+=s.prefix[s.valid]-s.prefix[at];
                } else {const Answer a=scan(x.data()+first,y.data()+first,last-first,q.threshold);result.count+=a.count;result.sum+=a.sum;}
                continue;
            }
            const PlaneBlock& p=planes[block];std::array<uint64_t,WORDS> mask{};
            const size_t lo=first-start,hi=last-start;bool progressive=true;
            const uint32_t t=q.threshold<=z.minimum?0:static_cast<uint32_t>(int64_t(q.threshold)-z.minimum);
            if(method=="bitplane_adaptive"&&t) {
                const unsigned bin=t>>(p.wx>4?p.wx-4:0);
                const uint64_t word=p.data[(2+p.wx+p.wy)*WORDS+bin/4];
                progressive=((word>>(16*(bin%4)))&65535)<=p.rows/8;
            }
            for(size_t word=lo/64;word<(hi+63)/64;++word) {
                const unsigned from=static_cast<unsigned>(std::max(lo,word*64)-word*64),to=static_cast<unsigned>(std::min(hi,(word+1)*64)-word*64);
                const uint64_t valid=p.data[word]&low(to)&~low(from);uint64_t matched=valid;
                if(t) {
                    uint64_t equal=valid,greater=0;
                    for(unsigned k=p.wx;k>0;--k) {
                        const unsigned j=k-1;const uint64_t ones=p.data[2*WORDS+j*WORDS+word];
                        if((t>>j)&1)equal&=ones;else {greater|=equal&ones;equal&=~ones;}
                        if(progressive&&(!equal||!(t&low(j))))break;
                    }
                    matched=greater|equal;
                }
                result.count+=popcount(matched);mask[word]=matched&p.data[WORDS+word];result.sum+=int64_t(p.ybase)*popcount(mask[word]);
            }
            for(unsigned bit=0;bit<p.wy;++bit) {
                unsigned count=0;for(size_t word=lo/64;word<(hi+63)/64;++word)count+=popcount(mask[word]&p.data[(2+p.wx+bit)*WORDS+word]);
                result.sum+=int64_t(uint64_t(1)<<bit)*count;
            }
        }
        return result;
    }
    size_t capacity() const {
        size_t bytes=sizeof(*this)+4*(x.capacity()+y.capacity())+index.capacity()*sizeof(Bounds)
            +planes.capacity()*sizeof(PlaneBlock)+sorted.capacity()*sizeof(SortedBlock)+thresholds.capacity()*sizeof(int32_t)
            +events.capacity()*sizeof(Event)+8*(tree_count.capacity()+tree_sum.capacity());
        for(const auto& p:planes)bytes+=p.words*8;
        for(const auto& s:sorted)bytes+=s.valid*4+(s.valid+1)*8;
        return bytes;
    }
};

uint64_t rss() {
#if defined(__unix__) || defined(__APPLE__)
    rusage usage{};if(getrusage(RUSAGE_SELF,&usage)!=0)return 0;
#if defined(__APPLE__)
    return static_cast<uint64_t>(usage.ru_maxrss);
#else
    return static_cast<uint64_t>(usage.ru_maxrss)*1024;
#endif
#else
    return 0;
#endif
}
struct InputInfo {size_t source_rows=0,bytes=0;uint64_t projected_fnv=UINT64_C(14695981039346656037);};
InputInfo ingest(const Options& o,const std::vector<Query>& queries,Store& store) {
    std::ifstream input(o.input,std::ios::binary);if(!input)throw std::runtime_error("Cannot open source input");
    const char delimiter=o.format=="household"?';':',';size_t xi=0,yi=5,expected_fields=55;
    std::string line;InputInfo info;
    if(o.format!="covertype") {
        if(!std::getline(input,line))throw std::runtime_error("Missing CSV header");
        info.bytes+=line.size()+1;if(!line.empty()&&line.back()=='\r')line.pop_back();
        const std::string xname=o.format=="household"?"Global_active_power":"cnt",yname=o.format=="household"?"Global_reactive_power":"registered";
        bool xfound=false,yfound=false;size_t start=0,field=0;
        do {const auto next=line.find(delimiter,start);const auto value=std::string_view(line).substr(start,next==std::string::npos?line.size()-start:next-start);
            if(value==xname){xi=field;xfound=true;}if(value==yname){yi=field;yfound=true;}++field;
            if(next==std::string::npos)break;start=next+1;
        }while(true);
        if(!xfound||!yfound||xi==yi)throw std::runtime_error("Required columns absent");expected_fields=field;
    }
    std::array<int32_t,BLOCK> chunk_x{},chunk_y{};size_t count=0;
    while(std::getline(input,line)) {
        info.bytes+=line.size()+(input.eof()?0:1);if(!line.empty()&&line.back()=='\r')line.pop_back();
        if(line.size()>1048576)throw std::runtime_error("Source record too long");
        size_t start=0,field=0;int32_t x=MISSING,y=MISSING;
        do {const auto next=line.find(delimiter,start);if(field==xi||field==yi) {
                const auto value=std::string_view(line).substr(start,next==std::string::npos?line.size()-start:next-start);
                const int32_t parsed=integer(value,o.format=="household");if(field==xi)x=parsed;else y=parsed;
            }
            ++field;if(next==std::string::npos)break;start=next+1;
        }while(true);
        if(field!=expected_fields)throw std::runtime_error("Unexpected source field count");
        if(info.source_rows>=o.skip) {
            if(info.source_rows-o.skip>=o.rows)throw std::runtime_error("More source rows than declared");
            for(int32_t v:{x,y})for(unsigned b=0;b<4;++b){info.projected_fnv^=(static_cast<uint32_t>(v)>>(8*b))&255;info.projected_fnv*=UINT64_C(1099511628211);}
            if(o.method=="stream_sweep")store.sweep_row(x,y,queries);
            else {chunk_x[count]=x;chunk_y[count]=y;if(++count==BLOCK){store.add(chunk_x.data(),chunk_y.data(),count);count=0;}}
        }
        ++info.source_rows;
    }
    if(input.bad()||info.source_rows!=o.rows+o.skip)throw std::runtime_error("Source row count differs from declaration");
    if(count)store.add(chunk_x.data(),chunk_y.data(),count);
    if(o.method=="stream_sweep"){store.event_prefix(o.rows,queries);if(store.next_event!=store.events.size())throw std::runtime_error("Unconsumed endpoint events");}
    if(store.n!=o.rows)throw std::runtime_error("Internal row count mismatch");return info;
}
} // namespace

int main(int argc,char** argv) try {
    const auto application_start=Clock::now();const Options o=parse_options(argc,argv);
    const auto queries=read_queries(o);std::vector<Answer> answers(queries.size());
    const auto setup_end=Clock::now();Store store(o,queries,answers);
    const InputInfo info=ingest(o,queries,store);const auto ingestion_end=Clock::now();const auto ingestion_rss=rss();
    if(o.method!="stream_sweep")for(size_t i=0;i<queries.size();++i)answers[i]=store.query(queries[i]);
    const auto queries_end=Clock::now();
    std::ofstream out(o.output,std::ios::binary);if(!out)throw std::runtime_error("Cannot open output report");
    out<<std::setprecision(12)<<"{\n\"schema_version\":1,\"experiment\":\"offline_exact_report\",\"method\":"<<quote(o.method)
       <<",\"input_name\":"<<quote(basename(o.input))<<",\"format\":"<<quote(o.format)<<",\"evaluation_rows\":"<<o.rows
       <<",\"skipped_rows\":"<<o.skip<<",\"source_rows\":"<<info.source_rows<<",\"source_bytes\":"<<info.bytes
       <<",\"query_count\":"<<queries.size()<<",\"block_rows\":"<<BLOCK<<",\"projected_fnv1a64\":"<<quote(std::to_string(info.projected_fnv))
       <<",\"semantics\":\"Count nonmissing x >= threshold; exact int64 sum of corresponding nonmissing y; [begin,end); empty sum zero\""
       <<",\"execution_contract\":\"Entire query plan known before input; immutable source; no cached complete query answers\""
       <<",\"raw_kernel\":"<<quote(
#ifdef REPORT_NEON
           "ARM NEON with widening int64 accumulation"
#else
           "Portable compiler-optimizable scalar loop"
#endif
       )<<",\"streaming_ingestion\":true,\"sweep_queries_fused_into_ingestion\":"<<(o.method=="stream_sweep"?"true":"false")
       <<",\"memory\":{\"store_capacity_bytes\":"<<store.capacity()<<",\"query_plan_capacity_bytes\":"<<queries.capacity()*sizeof(Query)
       <<",\"answers_capacity_bytes\":"<<answers.capacity()*sizeof(Answer)<<",\"bounded_input_chunk_bytes\":"<<2*BLOCK*sizeof(int32_t)
       <<",\"accounting_note\":\"Owned store capacities and objects; excludes allocator headers, I/O buffers, source line, options, executable/library pages; process peak RSS is separate\"}"
       <<",\"timing_ms\":{\"options_and_query_plan\":"<<elapsed(application_start,setup_end)
       <<",\"store_setup_source_parse_and_build\":"<<elapsed(setup_end,ingestion_end)
       <<",\"post_ingestion_queries\":"<<elapsed(ingestion_end,queries_end)<<"}"
       <<",\"rss_after_ingestion_bytes\":"<<ingestion_rss<<",\"answers\":[";
    for(size_t i=0;i<queries.size();++i){if(i)out<<',';out<<"{\"begin\":"<<queries[i].begin<<",\"end\":"<<queries[i].end<<",\"threshold\":"<<queries[i].threshold<<",\"count\":"<<answers[i].count<<",\"sum\":"<<answers[i].sum<<'}';}
    out.flush();if(!out)throw std::runtime_error("Report serialization failed");
    const auto serialization_end=Clock::now();
    out<<"],\"application_ms_to_answer_flush\":"<<elapsed(application_start,serialization_end)
       <<",\"answer_serialization_ms\":"<<elapsed(queries_end,serialization_end)<<",\"process_peak_rss_bytes\":"<<rss()
       <<",\"timing_note\":\"External process wall time is primary and includes startup, source I/O, construction, all queries, output, final flush and exit; internal diagnostics stop before final metadata/close\"\n}\n";
    out.close();if(!out)throw std::runtime_error("Report close failed");return 0;
} catch(const std::exception& error){std::cerr<<"report-bench: "<<error.what()<<'\n';return 1;}
