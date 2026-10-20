#version 330 core
uniform vec4 back_color;
uniform vec4 source_color;
uniform float opacity;
uniform int mode;
uniform int steps;
uniform float quant_max;
out vec4 result;
float burn(float b,float s){return b==1.?1.:(s==0.?0.:1.-min(1.,(1.-b)/s));}
float dodge(float b,float s){return b==0.?0.:(s==1.?1.:min(1.,b/(1.-s)));}
float vivid(float b,float s){return s<=.5?burn(b,2.*s):dodge(b,2.*s-1.);}
float channel(float b,float s){
    if(mode==0)return s;
    if(mode==1)return min(b,s);
    if(mode==2)return b*s;
    if(mode==3)return burn(b,s);
    if(mode==4)return max(0.,b+s-1.);
    if(mode==5)return max(0.,b-s);
    if(mode==7)return max(b,s);
    if(mode==8)return b+s-b*s;
    if(mode==9)return dodge(b,s);
    if(mode==10)return max(b,s==0.?0.:(b==1.?1.:min(1.,s*s/(1.-b))));
    if(mode==11)return min(1.,b+s);
    if(mode==12)return b+s;
    if(mode==14)return b<=.5?2.*b*s:1.-2.*(1.-b)*(1.-s);
    if(mode==15){float d=b<=.25?((16.*b-12.)*b+4.)*b:sqrt(b);return s<=.5?b-(1.-2.*s)*b*(1.-b):b+(2.*s-1.)*(d-b);}
    if(mode==16)return s<=.5?2.*b*s:1.-2.*(1.-b)*(1.-s);
    if(mode==17)return vivid(b,s);
    if(mode==18)return clamp(b+2.*s-1.,0.,1.);
    if(mode==19)return s<=.5?min(b,2.*s):max(b,2.*s-1.);
    if(mode==20)return vivid(b,s)<.5?0.:1.;
    if(mode==21)return abs(b-s);
    if(mode==22)return b+s-2.*b*s;
    if(mode==27)return s==0.?1.:min(1.,b/s);
    return s;
}
float lum(vec3 c){return dot(c,vec3(.3,.59,.11));}
float saturation(vec3 c){return max(c.r,max(c.g,c.b))-min(c.r,min(c.g,c.b));}
vec3 set_sat(vec3 c,float s){float lo=min(c.r,min(c.g,c.b));float hi=max(c.r,max(c.g,c.b));return hi>lo?(c-lo)*s/(hi-lo):vec3(0.);}
vec3 set_lum(vec3 c,float l){
    c+=l-lum(c);float lo=min(c.r,min(c.g,c.b));float hi=max(c.r,max(c.g,c.b));
    if(lo<0.)c=l+(c-l)*l/(l-lo);
    if(hi>1.)c=l+(c-l)*(1.-l)/(hi-l);
    return clamp(c,0.,1.);
}
vec3 blend(vec3 b,vec3 s){
    b=clamp(b,0.,1.);s=clamp(s,0.,1.);
    if(mode==6)return lum(b)<=lum(s)?b:s;
    if(mode==13)return lum(b)>=lum(s)?b:s;
    if(mode==23)return set_lum(set_sat(s,saturation(b)),lum(b));
    if(mode==24)return set_lum(set_sat(b,saturation(s)),lum(b));
    if(mode==25)return set_lum(s,lum(b));
    if(mode==26)return set_lum(b,lum(s));
    return vec3(channel(b.r,s.r),channel(b.g,s.g),channel(b.b,s.b));
}
void main(){
    vec4 b=back_color;float a=source_color.a*opacity;
    for(int i=0;i<steps;i++){
        float alpha=a+b.a*(1.-a);
        if(alpha==0.){b=vec4(0.);continue;}
        vec3 rgb=((1.-a)*b.rgb*b.a+(1.-b.a)*source_color.rgb*a+a*b.a*blend(b.rgb,source_color.rgb))/alpha;
        b=vec4(clamp(rgb,0.,1.),alpha);
    }
    vec4 stored=floor(clamp(b,0.,1.)*quant_max+0.5);
    result=stored.a==0.?vec4(0.):stored;
}
