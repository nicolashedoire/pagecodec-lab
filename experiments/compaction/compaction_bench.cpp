// User-space minor-compaction mechanism experiment; not a kernel implementation.
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <ctime>
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
#include <sys/mman.h>
#include <sys/resource.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <mach/mach.h>
#endif

namespace {
using Clock=std::chrono::steady_clock;
constexpr uint64_t FNV_OFFSET=UINT64_C(14695981039346656037),FNV_PRIME=UINT64_C(1099511628211);
constexpr size_t ALIGNMENT=16,MAX_MAPPING=512ULL*1024*1024,MAX_RECORDS=2000000;
struct Options {std::string input,policy="fifo",layout="random";uint64_t seed=20261030;size_t copies=1,segment=65536,scrub_mib=0;double fraction=.5;};
double ms(Clock::time_point a,Clock::time_point b){return std::chrono::duration<double,std::milli>(b-a).count();}
size_t round_up(size_t n,size_t alignment){return ((n+alignment-1)/alignment)*alignment;}
uint64_t number(const std::string& s){if(s.empty()||s[0]=='-')throw std::runtime_error("Invalid unsigned integer");size_t used=0;const auto n=std::stoull(s,&used);if(used!=s.size())throw std::runtime_error("Invalid unsigned integer");return n;}
Options options(int argc,char** argv){
    Options o;
    for(int i=1;i<argc;++i){const std::string k=argv[i];if(k=="--help"){std::cout<<"compaction-bench --input PACKED --policy fifo|benefit|efficiency|none --layout random|front|tail|mixed|subpage|none [--seed N --copies N --segment-bytes 65536 --reclaim-fraction 0.5 --cache-scrub-mib 0]\n";std::exit(0);}if(i+1==argc)throw std::runtime_error("Missing option value");const std::string v=argv[++i];
        if(k=="--input")o.input=v;else if(k=="--policy")o.policy=v;else if(k=="--layout")o.layout=v;
        else if(k=="--seed")o.seed=number(v);else if(k=="--copies")o.copies=number(v);else if(k=="--segment-bytes")o.segment=number(v);else if(k=="--cache-scrub-mib")o.scrub_mib=number(v);
        else if(k=="--reclaim-fraction"){size_t n=0;o.fraction=std::stod(v,&n);if(n!=v.size())throw std::runtime_error("Invalid reclaim fraction");}
        else throw std::runtime_error("Unknown option");}
    if(o.input.empty()||!o.copies||o.copies>1024||o.segment<4096||o.segment>16*1024*1024||o.scrub_mib>128||!std::isfinite(o.fraction)||o.fraction<=0||o.fraction>1)throw std::runtime_error("Options exceed supported bounds");
    if(o.policy!="fifo"&&o.policy!="benefit"&&o.policy!="efficiency"&&o.policy!="none")throw std::runtime_error("Unknown policy");
    if(o.layout!="random"&&o.layout!="front"&&o.layout!="tail"&&o.layout!="mixed"&&o.layout!="subpage"&&o.layout!="none")throw std::runtime_error("Unknown deletion layout");return o;
}
std::string quote(const std::string& s){std::ostringstream o;o<<'"';for(unsigned char c:s){if(c=='"'||c=='\\')o<<'\\'<<c;else if(c<32)o<<"\\u"<<std::hex<<std::setw(4)<<std::setfill('0')<<unsigned(c)<<std::dec;else o<<c;}o<<'"';return o.str();}
std::string hex(uint64_t n){std::ostringstream s;s<<std::hex<<std::setw(16)<<std::setfill('0')<<n;return s.str();}
void hash_bytes(uint64_t& hash,const unsigned char* p,size_t n){for(size_t i=0;i<n;++i){hash^=p[i];hash*=FNV_PRIME;}}
void hash_u64(uint64_t& hash,uint64_t n){for(unsigned i=0;i<8;++i){hash^=(n>>(8*i))&255;hash*=FNV_PRIME;}}
uint32_t read_u32(std::istream& in){unsigned char b[4];if(!in.read(reinterpret_cast<char*>(b),4))throw std::runtime_error("Truncated packed record header");return uint32_t(b[0])|(uint32_t(b[1])<<8)|(uint32_t(b[2])<<16)|(uint32_t(b[3])<<24);}
uint64_t resident(size_t page){
#if defined(__APPLE__)
    mach_task_basic_info_data_t info{};mach_msg_type_number_t count=MACH_TASK_BASIC_INFO_COUNT;
    if(task_info(mach_task_self(),MACH_TASK_BASIC_INFO,reinterpret_cast<task_info_t>(&info),&count)!=KERN_SUCCESS)throw std::runtime_error("Mach resident-memory query failed");
    (void)page;return info.resident_size;
#elif defined(__linux__)
    std::ifstream in("/proc/self/statm");uint64_t total=0,rss=0;if(!(in>>total>>rss))throw std::runtime_error("Cannot read Linux resident memory");return rss*page;
#else
    (void)page;throw std::runtime_error("Resident-memory observation unsupported on this OS");
#endif
}
uint64_t peak_rss(){rusage r{};if(getrusage(RUSAGE_SELF,&r))throw std::runtime_error("getrusage failed");
#if defined(__APPLE__)
    return static_cast<uint64_t>(r.ru_maxrss);
#else
    return static_cast<uint64_t>(r.ru_maxrss)*1024;
#endif
}
struct Record {uint64_t handle=0;size_t offset=0;uint32_t size=0,raw=0;bool live=true;size_t span() const{return round_up(size,ALIGNMENT);}};
struct Segment {
    unsigned char* data=nullptr;size_t mapped=0,used=0,live_bytes=0,deleted_bytes=0,first_hole=std::numeric_limits<size_t>::max();
    std::vector<Record> records;
    explicit Segment(size_t bytes){void* p=mmap(nullptr,bytes,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANON,-1,0);if(p==MAP_FAILED)throw std::runtime_error("mmap failed");data=static_cast<unsigned char*>(p);mapped=bytes;}
    Segment(const Segment&)=delete;Segment& operator=(const Segment&)=delete;
    ~Segment(){if(data&&mapped)munmap(data,mapped);}
    void trim(size_t bytes){if(bytes>mapped)throw std::runtime_error("Invalid mapping trim");if(bytes==mapped)return;if(munmap(data+bytes,mapped-bytes))throw std::runtime_error("munmap failed");mapped=bytes;if(!bytes)data=nullptr;}
};
struct Pool {
    std::vector<std::unique_ptr<Segment>> segments;
    size_t logical_page=0,source_records=0,total_records=0,source_logical=0,source_compressed=0;
    size_t initial_tail_trimmed=0,common_empty_released=0,live_records=0,deleted_records=0;
    size_t live_payload=0,deleted_payload=0,initial_mapped=0;uint64_t source_hash=0;
    size_t mapped() const{size_t n=0;for(const auto& s:segments)n+=s->mapped;return n;}
    size_t metadata() const{size_t n=sizeof(*this)+segments.capacity()*sizeof(std::unique_ptr<Segment>);for(const auto& s:segments)n+=sizeof(Segment)+s->records.capacity()*sizeof(Record);return n;}
};
Pool build(const Options& o,size_t page){
    Pool pool;std::vector<unsigned char> buffer;uint64_t handle=0;
    for(size_t copy=0;copy<o.copies;++copy){
        std::ifstream input(o.input,std::ios::binary);if(!input)throw std::runtime_error("Cannot open packed source");
        std::array<char,8> magic{};if(!input.read(magic.data(),8)||magic!=std::array<char,8>{'P','C','M','P','0','0','1','\0'})throw std::runtime_error("Invalid packed magic");
        const uint32_t logical=read_u32(input),count=read_u32(input);
        if(!logical||logical>1024*1024||!count||count>MAX_RECORDS/o.copies)throw std::runtime_error("Packed source counts exceed bounds");
        if(copy&&(logical!=pool.logical_page||count!=pool.source_records))throw std::runtime_error("Source changed between copies");
        if(!copy){pool.logical_page=logical;pool.source_records=count;}
        size_t logical_sum=0,compressed_sum=0;uint64_t source_hash=FNV_OFFSET;
        hash_u64(source_hash,logical);hash_u64(source_hash,count);
        for(uint32_t i=0;i<count;++i){
            const uint32_t raw=read_u32(input),size=read_u32(input);const size_t span=round_up(size,ALIGNMENT);
            if(!raw||raw>logical||!size||span>o.segment)throw std::runtime_error("Invalid packed record length");
            buffer.resize(size);if(!input.read(reinterpret_cast<char*>(buffer.data()),size))throw std::runtime_error("Truncated packed payload");
            hash_u64(source_hash,raw);hash_u64(source_hash,size);hash_bytes(source_hash,buffer.data(),size);
            if(pool.segments.empty()||pool.segments.back()->used+span>o.segment){
                if(pool.segments.size()>=MAX_MAPPING/o.segment)throw std::runtime_error("Pool mapping safety limit exceeded");
                pool.segments.push_back(std::make_unique<Segment>(o.segment));
            }
            auto& s=*pool.segments.back();std::memcpy(s.data+s.used,buffer.data(),size);std::memset(s.data+s.used+size,0,span-size);
            s.records.push_back({handle++,s.used,size,raw,true});s.used+=span;s.live_bytes+=span;
            logical_sum+=raw;compressed_sum+=size;
        }
        if(input.peek()!=std::char_traits<char>::eof())throw std::runtime_error("Unexpected trailing packed bytes");
        if(!copy){pool.source_logical=logical_sum;pool.source_compressed=compressed_sum;pool.source_hash=source_hash;}
        else if(logical_sum!=pool.source_logical||compressed_sum!=pool.source_compressed||source_hash!=pool.source_hash)throw std::runtime_error("Source payload changed between copies");
    }
    pool.total_records=handle;
    for(auto& s:pool.segments){const size_t kept=round_up(s->used,page);pool.initial_tail_trimmed+=s->mapped-kept;s->trim(kept);}
    pool.initial_mapped=pool.mapped();return pool;
}
size_t reclaimable(const Segment& s,size_t page){
    if(!s.mapped||!s.deleted_bytes||s.mapped-s.live_bytes<page)return 0;
    return s.mapped-round_up(s.live_bytes,page);
}
void erase_record(Pool& pool,Segment& s,size_t i){auto& r=s.records[i];if(!r.live)throw std::runtime_error("Duplicate deletion");r.live=false;s.first_hole=std::min(s.first_hole,i);s.live_bytes-=r.span();s.deleted_bytes+=r.span();++pool.deleted_records;pool.deleted_payload+=r.size;}
void delete_records(Pool& pool,const Options& o,size_t page){
    std::mt19937_64 rng(o.seed);
    for(size_t si=0;si<pool.segments.size();++si){auto& s=*pool.segments[si];std::vector<size_t> order(s.records.size());std::iota(order.begin(),order.end(),0);
        double fraction=.45;std::string direction=o.layout;
        if(o.layout=="none")fraction=0;
        if(o.layout=="mixed"){constexpr double fractions[]={0,.15,.35,.65};fraction=fractions[si%4];direction=((si+si/4)%2)?"front":"tail";}
        if(direction=="tail")std::reverse(order.begin(),order.end());
        if(direction=="random"||direction=="subpage")for(size_t n=order.size();n>1;--n)std::swap(order[n-1],order[static_cast<size_t>(rng()%n)]);
        if(o.layout=="subpage"){
            for(size_t i:order){const size_t remaining=s.live_bytes-s.records[i].span();if(remaining&&s.mapped-round_up(remaining,page)==0)erase_record(pool,s,i);}
        }else {
            const size_t wanted=static_cast<size_t>(std::floor(s.used*fraction));
            for(size_t i:order){if(s.deleted_bytes>=wanted)break;erase_record(pool,s,i);}
        }
        if(!s.live_bytes&&s.mapped){pool.common_empty_released+=s.mapped;s.trim(0);}
        for(const auto& r:s.records)if(r.live){++pool.live_records;pool.live_payload+=r.size;}
    }
}
uint64_t checksum(const Pool& pool){
    uint64_t hash=FNV_OFFSET;
    for(const auto& s:pool.segments)for(const auto& r:s->records)if(r.live){
        if(!s->data||r.offset+r.span()>s->mapped)throw std::runtime_error("Live handle outside mapping");
        hash_u64(hash,r.handle);hash_u64(hash,r.size);hash_bytes(hash,s->data+r.offset,r.size);
    }
    return hash;
}
struct Candidate {size_t index=0,reclaim=0,moved=0;};
struct Outcome {
    size_t moved=0,payload_moved=0,records_moved=0,released=0,segments_compacted=0,metadata=0,prediction_records=0;
    double decision=-1,elapsed=0,cpu=0;
};
Outcome compact(Pool& pool,const Options& o,size_t page,size_t target){
    Outcome result;const auto start=Clock::now();const auto cpu=std::clock();
    auto execute=[&](const Candidate& candidate){auto& s=*pool.segments[candidate.index];size_t destination=s.records[s.first_hole].offset;
        for(size_t i=s.first_hole;i<s.records.size();++i){auto& r=s.records[i];if(r.live){if(r.offset!=destination){std::memmove(s.data+destination,s.data+r.offset,r.span());result.moved+=r.span();result.payload_moved+=r.size;++result.records_moved;}r.offset=destination;destination+=r.span();}}
        if(destination!=s.live_bytes)throw std::runtime_error("Compacted byte accounting mismatch");
        const size_t kept=round_up(destination,page),released=s.mapped-kept;
        if(released!=candidate.reclaim)throw std::runtime_error("Reclaim predictor disagrees with actual trim");
        s.trim(kept);
        result.released+=released;++result.segments_compacted;
    };
    if(o.policy=="fifo"){
        for(size_t i=0;i<pool.segments.size()&&result.released<target;++i){const size_t gain=reclaimable(*pool.segments[i],page);if(gain)execute({i,gain,0});}
        result.elapsed=ms(start,Clock::now());
        result.cpu=1000.0*double(std::clock()-cpu)/CLOCKS_PER_SEC;return result;
    }
    std::vector<Candidate> candidates;size_t selected=0,selected_reclaim=0;
    if(target&&o.policy!="none"){
        for(size_t i=0;i<pool.segments.size();++i){const auto& s=*pool.segments[i];const size_t gain=reclaimable(s,page);if(!gain)continue;
            size_t copied=0;
            if(o.policy=="efficiency"){size_t destination=s.records[s.first_hole].offset;for(size_t j=s.first_hole;j<s.records.size();++j){const auto& r=s.records[j];++result.prediction_records;if(r.live){if(r.offset!=destination)copied+=r.span();destination+=r.span();}}}
            candidates.push_back({i,gain,copied});
        }
        if(o.policy=="benefit")std::sort(candidates.begin(),candidates.end(),[](const Candidate& a,const Candidate& b){return a.reclaim!=b.reclaim?a.reclaim>b.reclaim:a.index<b.index;});
        if(o.policy=="efficiency")std::sort(candidates.begin(),candidates.end(),[page](const Candidate& a,const Candidate& b){const uint64_t left=uint64_t(a.reclaim)*(b.moved+page),right=uint64_t(b.reclaim)*(a.moved+page);return left!=right?left>right:a.index<b.index;});
        selected_reclaim=0;while(selected<candidates.size()&&selected_reclaim<target)selected_reclaim+=candidates[selected++].reclaim;
    }
    result.metadata=candidates.capacity()*sizeof(Candidate);if(o.policy!="none")result.decision=ms(start,Clock::now());
    for(size_t i=0;i<selected;++i)execute(candidates[i]);
    std::vector<Candidate>().swap(candidates); // Include decision-storage release in measured work.
    result.elapsed=ms(start,Clock::now());result.cpu=1000.0*double(std::clock()-cpu)/CLOCKS_PER_SEC;return result;
}
} // namespace

int main(int argc,char** argv) try {
    const auto setup_start=Clock::now();const Options o=options(argc,argv);const long os_page=sysconf(_SC_PAGESIZE);
    if(os_page<=0)throw std::runtime_error("Cannot determine OS page size");const size_t page=static_cast<size_t>(os_page);
    if(o.segment%page)throw std::runtime_error("Segment size must be a multiple of OS page size");
    Pool pool=build(o,page);delete_records(pool,o,page);const auto setup_end=Clock::now();
    const auto target_start=Clock::now();size_t maximum=0,eligible=0;
    for(const auto& s:pool.segments){const size_t n=reclaimable(*s,page);maximum+=n;eligible+=n!=0;}
    const size_t pages=maximum/page;size_t target_pages=static_cast<size_t>(std::floor(pages*o.fraction));if(pages&&!target_pages)target_pages=1;const size_t target=target_pages*page;
    const auto target_end=Clock::now(),check_start=Clock::now();const uint64_t before=checksum(pool);const auto check_end=Clock::now();
    const auto conditioning_start=Clock::now();const size_t conditioning_bytes=o.scrub_mib*1024*1024;
    std::unique_ptr<unsigned char[]> conditioning;
    if(conditioning_bytes){conditioning.reset(new unsigned char[conditioning_bytes]);volatile unsigned char* p=conditioning.get();for(size_t i=0;i<conditioning_bytes;i+=64)p[i]=static_cast<unsigned char>(i/64);}
    const auto conditioning_end=Clock::now();
    const size_t mapped_before=pool.mapped();const uint64_t rss_before=resident(page);
    const Outcome outcome=compact(pool,o,page,target);
    const uint64_t rss_after=resident(page);
    const auto after_check_start=Clock::now();const uint64_t after=checksum(pool);const auto after_check_end=Clock::now();
    if(before!=after)throw std::runtime_error("Live compressed byte checksum changed");
    const size_t mapped_after=pool.mapped();if(mapped_before-mapped_after!=outcome.released)throw std::runtime_error("Mapping accounting mismatch");
    const uint64_t peak=peak_rss();
    std::cout<<std::setprecision(12)<<"{\"schema_version\":1,\"experiment\":\"user_space_minor_compaction\",\"policy\":"<<quote(o.policy)<<",\"layout\":"<<quote(o.layout)
      <<",\"seed\":"<<o.seed<<",\"copies\":"<<o.copies<<",\"reclaim_fraction\":"<<o.fraction<<",\"page_bytes\":"<<page<<",\"segment_bytes\":"<<o.segment<<",\"record_alignment_bytes\":"<<ALIGNMENT
      <<",\"logical_page_bytes\":"<<pool.logical_page<<",\"source_record_count\":"<<pool.source_records<<",\"total_record_count\":"<<pool.total_records<<",\"source_logical_bytes\":"<<pool.source_logical<<",\"source_compressed_bytes\":"<<pool.source_compressed
      <<",\"source_fnv1a64\":"<<quote(hex(pool.source_hash))<<",\"segment_count\":"<<pool.segments.size()<<",\"eligible_count\":"<<eligible<<",\"live_record_count\":"<<pool.live_records<<",\"deleted_record_count\":"<<pool.deleted_records
      <<",\"live_payload_bytes\":"<<pool.live_payload<<",\"deleted_payload_bytes\":"<<pool.deleted_payload<<",\"initial_mapped_bytes\":"<<pool.initial_mapped<<",\"initial_tail_trimmed_bytes\":"<<pool.initial_tail_trimmed
      <<",\"common_empty_released_bytes\":"<<pool.common_empty_released<<",\"mapped_before\":"<<mapped_before<<",\"mapped_after\":"<<mapped_after<<",\"released_bytes\":"<<outcome.released
      <<",\"max_reclaimable_bytes\":"<<maximum<<",\"target_bytes\":"<<target<<",\"target_met\":"<<(outcome.released>=target?"true":"false")<<",\"overshoot_bytes\":"<<(outcome.released>target?outcome.released-target:0)
      <<",\"bytes_moved\":"<<outcome.moved<<",\"payload_bytes_moved\":"<<outcome.payload_moved<<",\"records_moved\":"<<outcome.records_moved<<",\"segments_compacted\":"<<outcome.segments_compacted
      <<",\"metadata_bytes\":"<<pool.metadata()<<",\"decision_metadata_bytes\":"<<outcome.metadata<<",\"prediction_records_examined\":"<<outcome.prediction_records<<",\"efficiency_fixed_cost_bytes\":"<<page
      <<",\"setup_ms\":"<<ms(setup_start,setup_end)<<",\"target_scan_ms\":"<<ms(target_start,target_end)<<",\"checksum_ms\":"<<ms(check_start,check_end)+ms(after_check_start,after_check_end)
      <<",\"conditioning_ms\":"<<ms(conditioning_start,conditioning_end)<<",\"conditioning_bytes\":"<<conditioning_bytes
      <<",\"decision_ms\":"<<(outcome.decision<0?"null":std::to_string(outcome.decision))<<",\"compaction_ms\":null,\"reclaim_ms\":null,\"elapsed_ms\":"<<outcome.elapsed<<",\"cpu_ms\":"<<outcome.cpu
      <<",\"rss_before\":"<<rss_before<<",\"rss_after\":"<<rss_after<<",\"peak_rss_bytes\":"<<peak<<",\"checksum_before\":"<<quote(hex(before))<<",\"checksum_after\":"<<quote(hex(after))<<",\"checksum_ok\":true"
      <<",\"timing_scope\":\"Policy elapsed includes its own candidate scan/prediction/order, stable moves, handle updates, trailing munmap and candidate release. Setup, common quota scan, integrity checks and RSS observations are separate.\""
      <<",\"detail_timing_scope\":\"No per-segment clock calls. FIFO/none decision and movement/reclaim subphase timings are null; ranked decision uses one stage timestamp.\""
      <<",\"conditioning_scope\":\"Optional separate cache-line-write buffer allocated/touched after pre-checksum, excluded from policy time and held through both RSS snapshots. Shared conditioning is not application data and does not guarantee cold caches.\""
      <<",\"memory_scope\":\"Mapped byte release is exact address-space accounting; current RSS is observed separately and may differ. Peak RSS is process lifetime including setup.\""
      <<",\"layout_scope\":\"Actual compressed records, repeated only via explicit copies; synthetic deletions. 16-byte aligned records; initially unused whole pages and entirely empty segments released during common setup.\""
      <<",\"integrity_scope\":\"FNV-1a over stable handle ID, compressed length and every surviving payload byte before/after; noncryptographic error detector; no decompression or kernel integration.\"}\n";
    if(!std::cout)throw std::runtime_error("JSON output failed");return 0;
}catch(const std::exception& e){std::cerr<<"compaction-bench: "<<e.what()<<'\n';return 1;}
