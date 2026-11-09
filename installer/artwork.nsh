; Smooth the high-resolution artwork to the actual control size, including Windows DPI
; scaling. MUI's LoadImage resizing drops pixels; HALFTONE averages them instead. Keep the
; MUI-loaded bitmap as a fallback if a GDI allocation fails. RESULT is MUI's ownership handle
; on welcome/finish pages, so its normal page cleanup also releases the replacement.
!ifndef TA_ARTWORK_NSH
!define TA_ARTWORK_NSH

!macro TA_SMOOTH_BITMAP CONTROL FILE WIDTH HEIGHT RESULT
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  Push $R5
  Push $R6
  Push $R7
  Push $R8
  Push $R9
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $R0 ${CONTROL}
  System::Call '*(i0,i0,i0,i0)p.R3'
  ${If} $R3 P<> 0
    System::Call 'user32::GetClientRect(pR0,pR3)i.R9'
    System::Call '*$R3(i,i,i.R1,i.R2)'
    System::Free $R3
    ${If} $R9 <> 0
    ${AndIf} $R1 > 0
    ${AndIf} $R2 > 0
      System::Call 'user32::LoadImage(p0,t"${FILE}",i0,i0,i0,i0x2010)p.R3'
      System::Call 'user32::GetDC(pR0)p.R4'
      System::Call 'gdi32::CreateCompatibleDC(pR4)p.R5'
      System::Call 'gdi32::CreateCompatibleDC(pR4)p.R6'
      System::Call 'gdi32::CreateCompatibleBitmap(pR4,iR1,iR2)p.R7'
      ${If} $R3 P<> 0
      ${AndIf} $R4 P<> 0
      ${AndIf} $R5 P<> 0
      ${AndIf} $R6 P<> 0
      ${AndIf} $R7 P<> 0
        System::Call 'gdi32::SelectObject(pR5,pR3)p.R8'
        System::Call 'gdi32::SelectObject(pR6,pR7)p.R9'
        ${If} $R8 P<> 0
        ${AndIf} $R9 P<> 0
          System::Call 'gdi32::SetStretchBltMode(pR6,i4)i.R0'
          ${If} $R0 <> 0
            ; Dialog units vary with the UI font. Fit the complete panel without stretching
            ; the emblem; letterbox with MUI's gunmetal background (COLORREF is BGR).
            System::Call 'gdi32::CreateSolidBrush(i0x1A1511)p.r4'
            System::Call '*(i0,i0,iR1,iR2)p.r5'
            StrCpy $R0 0
            ${If} $4 P<> 0
            ${AndIf} $5 P<> 0
              System::Call 'user32::FillRect(pR6,pr5,pr4)i.R0'
            ${EndIf}
            ${If} $4 P<> 0
              System::Call 'gdi32::DeleteObject(pr4)'
            ${EndIf}
            ${If} $5 P<> 0
              System::Free $5
            ${EndIf}
          ${EndIf}
          ${If} $R0 <> 0
            System::Call 'kernel32::MulDiv(iR2,i${WIDTH},i${HEIGHT})i.r0'
            ${If} $0 <= $R1
              StrCpy $1 $R2
              IntOp $2 $R1 - $0
              IntOp $2 $2 / 2
              StrCpy $3 0
            ${Else}
              StrCpy $0 $R1
              System::Call 'kernel32::MulDiv(iR1,i${HEIGHT},i${WIDTH})i.r1'
              StrCpy $2 0
              IntOp $3 $R2 - $1
              IntOp $3 $3 / 2
            ${EndIf}
            System::Call 'gdi32::SetBrushOrgEx(pR6,i0,i0,p0)'
            System::Call 'gdi32::StretchBlt(pR6,ir2,ir3,ir0,ir1,pR5,i0,i0,i${WIDTH},i${HEIGHT},i0x00CC0020)i.R1'
          ${Else}
            StrCpy $R1 0
          ${EndIf}
          StrCpy $R0 ${CONTROL}
        ${Else}
          StrCpy $R1 0
        ${EndIf}
        ${If} $R8 P<> 0
          System::Call 'gdi32::SelectObject(pR5,pR8)'
        ${EndIf}
        ${If} $R9 P<> 0
          System::Call 'gdi32::SelectObject(pR6,pR9)'
        ${EndIf}
        ${If} $R1 <> 0
          SendMessage $R0 ${STM_SETIMAGE} ${IMAGE_BITMAP} $R7 $R1
          ${If} $R1 P<> 0
            System::Call 'gdi32::DeleteObject(pR1)'
          ${EndIf}
          StrCpy ${RESULT} $R7
          StrCpy $R7 0 ; The caller/MUI now owns this bitmap.
        ${EndIf}
      ${EndIf}
      ${If} $R7 P<> 0
        System::Call 'gdi32::DeleteObject(pR7)'
      ${EndIf}
      ${If} $R6 P<> 0
        System::Call 'gdi32::DeleteDC(pR6)'
      ${EndIf}
      ${If} $R5 P<> 0
        System::Call 'gdi32::DeleteDC(pR5)'
      ${EndIf}
      ${If} $R4 P<> 0
        System::Call 'user32::ReleaseDC(pR0,pR4)'
      ${EndIf}
      ${If} $R3 P<> 0
        System::Call 'gdi32::DeleteObject(pR3)'
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Pop $R9
  Pop $R8
  Pop $R7
  Pop $R6
  Pop $R5
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
!macroend

!endif
