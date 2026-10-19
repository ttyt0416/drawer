# 129 — 28개 혼합 모드의 고정 규격

공통 알파/기본 모드와 비분리 색 성분은 [W3C Compositing and Blending Level 1, 2024-03-21](https://www.w3.org/TR/2024/CRD-compositing-1-20240321/#blending)을 참조해 독립 구현했다. 추가 산술/명도 계열의 공개 개념은 [Krita Arithmetic](https://docs.krita.org/en/reference_manual/blending_modes/arithmetic.html), [Lighten](https://docs.krita.org/en/reference_manual/blending_modes/lighten.html), [Quadratic](https://docs.krita.org/en/reference_manual/blending_modes/quadratic.html)을 참고했다. 아래 수식 자체가 Drawer의 고정 규격이다. 발광 조합과 확장 모드를 타사 내부 계산과 동일하다고 주장하지 않는다. 프로그램은 이를 실행 중 검색/다운로드하지 않는다.

b/s는 배경/소스의 인코딩 sRGB 성분이며0..1이다. clip은0..1 제한이다. RGB 모드 외에는 채널별 계산한다. opacity와 마스크를 곱한 소스 알파 a, 배경 알파 d에 대해 출력 알파는 a+d(1-a), 미리 곱한 출력색은 (1-a)Cb*d+(1-d)Cs*a+B(Cb,Cs)*a*d이다. 각 레이어 경계에서 RGB를 출력 알파 범위로 제한하고 투명색은0이다. 클리핑은 동일 혼합을 사용하면서 기준 알파를 고정한다. Add (Glow)만 B가2까지 갈 수 있으며 최종 합성 단계에서 제한한다. F32 미리 곱하기를 푼 값의0/1 근방2e-7은 RGBA8의 정확한 경계로 복원해 Color burn의 반투명 흰색이 검게 잘못 바뀌는 일을 막는다.

| 모드 | B(b,s) 또는 RGB 규칙 |
|---|---|
| Normal | s |
| Darken | min(b,s) |
| Multiply | b*s |
| Color burn | b=1이면1, 그 외 s=0이면0, 나머지 1-min(1,(1-b)/s) |
| Linear burn | max(0,b+s-1) |
| Subtract | max(0,b-s) |
| Darker color | L이 작은 전체 RGB, 동률은 배경 |
| Lighten | max(b,s) |
| Screen | b+s-b*s |
| Color dodge | b=0이면0, 그 외 s=1이면1, 나머지 min(1,b/(1-s)) |
| Glow dodge | max(b,G), G는 s=0일 때0, 그 외 b=1이면1, 나머지 min(1,s*s/(1-b)) |
| Add | min(1,b+s), 알파 가중 전 제한 |
| Add (Glow) | b+s, 알파 합성 후 제한 |
| Lighter color | L이 큰 전체 RGB, 동률은 배경 |
| Overlay | b<=.5이면2*b*s, 나머지1-2*(1-b)*(1-s) |
| Soft light | s<=.5: b-(1-2s)b(1-b); 그 외 b+(2s-1)(D(b)-b) |
| Hard light | s<=.5이면2*b*s, 나머지1-2*(1-b)*(1-s) |
| Vivid light | s<=.5이면Burn(b,2s), 나머지Dodge(b,2s-1) |
| Linear light | clip(b+2s-1) |
| Pin light | s<=.5이면min(b,2s), 나머지max(b,2s-1) |
| Hard mix | Vivid light<.5이면0, 나머지1 |
| Difference | abs(b-s) |
| Exclusion | b+s-2*b*s |
| Hue | 소스 색상, 배경 채도/광도 |
| Saturation | 배경 색상/광도, 소스 채도 |
| Color | 소스 색상/채도, 배경 광도 |
| Brightness | 배경 색상/채도, 소스 광도 |
| Divide | s=0이면1, 나머지min(1,b/s) |

Soft light의 D(b)는 b<=.25에서 ((16b-12)b+4)b, 그 외 sqrt(b)이다. 이로써 Soft/Hard/Vivid/Linear/Pin을 별도 계산한다. 비분리 성분은 L=.3R+.59G+.11B, S=max-min을 사용한다. SetSat은 최소를0, 최대를원하는S로 선형 재배치하고 무채색은0으로 둔다. SetLum은 원하는L만큼 이동한 후 같은L 중심에서 색 범위를 압축한다. Hue는 SetLum(SetSat(Cs,S(Cb)),L(Cb)), Saturation은 SetLum(SetSat(Cb,S(Cs)),L(Cb)), Color는 SetLum(Cs,L(Cb)), Brightness는 SetLum(Cb,L(Cs))이다. 이 L은 회색 표현의54/183/19 밝기와 별도의 혼합 규칙이다.

알파0 소스/불투명도0은 배경을 유지하고 빈 배경은 소스색을 보존한다. ICC 프루프는 합성 후 화면 변환이며 계산 공간을 바꾸지 않는다. 원본 픽셀/기하와 혼합 설정을 분리하고 문서v57에 모드를 보존한다. PSD의 편집 가능한 레이어 저장은 일반 이외 모드가 있으면 쓰기 전에 거절하고, 실제 사용 모드 목록과 렌더된 전체 이미지 선택을 안내한다. 렌더된 PNG/PSD는 동일 합성을 사용한다. 미지원 모드를 Normal로 조용히 바꾸지 않는다.

독립 수치 검증은28개 회색 기준값·알파0/128/255·불투명도0/.5/1·빈 배경, 원색/흰색/검정/중성/반투명 격자와 고정 클리핑 알파를 포함한다. RGB 성분 교체의 별도 값과 색역 제한, Light 다섯 모드의 .35/.8 차이, 가산230/발광255, 채널 비교/전체색 비교, 반투명 흰색 Burn170/170/170/192와 발광의 검정 중립을 확인한다. 모든 모드의 렌더·저장/재열기·Undo, PSD 거절 시 무파일·렌더된 PSD의 픽셀, ICC 켬/끔과 알파/문서 보존을 확인한다. 실제 스크롤 메뉴28개 선택/Apply/Undo와28개 CPU 렌더 결과의 GPU 업로드 텍스처 값을 검증한다. 물리 GPU 프레임버퍼의 비트 일치를 주장하지 않는다.
