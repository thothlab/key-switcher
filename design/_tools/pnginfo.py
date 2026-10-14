import sys, zlib, struct
def read(path):
    d=open(path,'rb').read(); assert d[:8]==b'\x89PNG\r\n\x1a\n'
    i=8; idat=b''; ihdr=None
    while i<len(d):
        n=struct.unpack('>I',d[i:i+4])[0]; t=d[i+4:i+8]; c=d[i+8:i+8+n]; i+=12+n
        if t==b'IHDR': ihdr=struct.unpack('>IIBBBBB',c)
        elif t==b'IDAT': idat+=c
    w,h,bd,ct=ihdr[:4]; bpp={6:4,2:3,0:1,4:2}[ct]
    raw=zlib.decompress(idat); stride=w*bpp; rows=[]; prev=bytearray(stride); p=0
    for y in range(h):
        f=raw[p]; p+=1; line=bytearray(raw[p:p+stride]); p+=stride
        for x in range(stride):
            a=line[x-bpp] if x>=bpp else 0; b=prev[x]; c=prev[x-bpp] if x>=bpp else 0
            if f==1: line[x]=(line[x]+a)&255
            elif f==2: line[x]=(line[x]+b)&255
            elif f==3: line[x]=(line[x]+(a+b)//2)&255
            elif f==4:
                pa=abs(b-c); pb=abs(a-c); pc=abs(a+b-2*c)
                pr=a if pa<=pb and pa<=pc else (b if pb<=pc else c); line[x]=(line[x]+pr)&255
        rows.append(bytes(line)); prev=line
    return w,h,ct,bpp,rows
if __name__=='__main__':
    for path in sys.argv[1:]:
        w,h,ct,bpp,rows=read(path)
        cols=set()
        for r in rows:
            for x in range(0,len(r),bpp):
                px=r[x:x+bpp]
                if bpp==4 and px[3]>0: cols.add(px[:3])
        print(path,w,h,'colortype',ct,'distinct opaque rgb',len(cols),sorted(cols)[:4])
