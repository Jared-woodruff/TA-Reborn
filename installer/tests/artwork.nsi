; Standalone, non-elevated regression for the production artwork macro. The nsDialogs
; page is never shown; GUI initialization writes the result and quits without installing.
Unicode true
RequestExecutionLevel user
ManifestDPIAware true
Name "TA Reborn artwork regression"
OutFile "${OUTPUT_FILE}"
!include MUI2.nsh
!include nsDialogs.nsh
!include LogicLib.nsh
!include "..\artwork.nsh"
!if ${NSIS_PTR_SIZE} > 4
  !define TEST_BITMAP_SIZE 32
!else
  !define TEST_BITMAP_SIZE 24
!endif

Var TestDialog
Var TestImage
Var TestBitmap
Var SavedBitmap
Var TestLog
Var Failures
Var ExpectedWidth
Var ExpectedHeight
Var ContentLeft
Var ContentTop
Var ContentRight
Var ContentBottom
Var TestDC
Var PreviousObject
Var SampleX
Var SampleY
Var Replacements
Var GdiBefore

!macro EXPECT_EQ ACTUAL EXPECTED DESCRIPTION
  ${If} ${ACTUAL} != ${EXPECTED}
    FileWrite $TestLog "FAIL: ${DESCRIPTION}: ${ACTUAL} != ${EXPECTED}$\r$\n"
    IntOp $Failures $Failures + 1
  ${EndIf}
!macroend

!define MUI_CUSTOMFUNCTION_GUIINIT RunArtworkTests
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro TEST_SCALE SCALE
  StrCpy $ExpectedWidth 16
  IntOp $ExpectedWidth $ExpectedWidth * ${SCALE}
  IntOp $ExpectedWidth $ExpectedWidth / 100
  StrCpy $ExpectedHeight 12
  IntOp $ExpectedHeight $ExpectedHeight * ${SCALE}
  IntOp $ExpectedHeight $ExpectedHeight / 100
  StrCpy $ContentLeft 0
  StrCpy $ContentTop 0
  StrCpy $ContentRight $ExpectedWidth
  StrCpy $ContentBottom $ExpectedHeight
  ; Explicit physical pixels exercise each scale without changing desktop DPI settings.
  System::Call 'user32::SetWindowPos(p$TestImage,p0,i0,i0,i$ExpectedWidth,i$ExpectedHeight,i0x16)i.r0'
  !insertmacro EXPECT_EQ $0 1 "resize ${SCALE}% control"
  !insertmacro TA_SMOOTH_BITMAP $TestImage "$PLUGINSDIR\checker.bmp" 256 192 $TestBitmap
  Call CheckBitmap
  FileWrite $TestLog "Checked ${SCALE}%: $ExpectedWidth x $ExpectedHeight pixels$\r$\n"
!macroend

!macro TEST_LETTERBOX WIDTH HEIGHT LEFT TOP RIGHT BOTTOM
  StrCpy $ExpectedWidth ${WIDTH}
  StrCpy $ExpectedHeight ${HEIGHT}
  StrCpy $ContentLeft ${LEFT}
  StrCpy $ContentTop ${TOP}
  StrCpy $ContentRight ${RIGHT}
  StrCpy $ContentBottom ${BOTTOM}
  System::Call 'user32::SetWindowPos(p$TestImage,p0,i0,i0,i$ExpectedWidth,i$ExpectedHeight,i0x16)i.r0'
  !insertmacro EXPECT_EQ $0 1 "resize letterbox control"
  !insertmacro TA_SMOOTH_BITMAP $TestImage "$PLUGINSDIR\checker.bmp" 256 192 $TestBitmap
  Call CheckBitmap
  FileWrite $TestLog "Checked centered aspect fit: ${WIDTH} x ${HEIGHT}, content [${LEFT},${TOP})..[${RIGHT},${BOTTOM})$\r$\n"
!macroend

Function RunArtworkTests
  StrCpy $Failures 0
  FileOpen $TestLog "${RESULT_FILE}" w
  ${If} $TestLog == ""
    SetErrorLevel 2
    Quit
  ${EndIf}
  InitPluginsDir
  File "/oname=$PLUGINSDIR\checker.bmp" "${FIXTURE_FILE}"
  nsDialogs::Create 1018
  Pop $TestDialog
  ${If} $TestDialog == error
    FileWrite $TestLog "FAIL: nsDialogs creation$\r$\n"
    FileClose $TestLog
    SetErrorLevel 2
    Quit
  ${EndIf}
  ${NSD_CreateBitmap} 0 0 16 12 ""
  Pop $TestImage
  ; Begin with a valid MUI-style fallback and its ownership handle.
  ${NSD_SetStretchedImage} $TestImage "$PLUGINSDIR\checker.bmp" $TestBitmap
  !insertmacro TEST_SCALE 100
  !insertmacro TEST_SCALE 125
  !insertmacro TEST_SCALE 150
  !insertmacro TEST_SCALE 200
  !insertmacro TEST_SCALE 300
  !insertmacro TEST_SCALE 400
  ; Font metrics can change the control aspect ratio. The whole source must keep its
  ; proportions and be centered, including asymmetric padding caused by integer rounding.
  !insertmacro TEST_LETTERBOX 64 64 0 8 64 56
  !insertmacro TEST_LETTERBOX 64 32 10 0 53 32

  ; Every successful replacement must release the previous bitmap and all temporary GDI
  ; objects. Sample after warmup so initialization caches cannot hide a per-call leak.
  System::Call 'kernel32::GetCurrentProcess()p.r0'
  System::Call 'user32::GetGuiResources(pr0,i0)i.r1'
  StrCpy $GdiBefore $1
  StrCpy $Replacements 0
  ${DoWhile} $Replacements < 64
    !insertmacro TA_SMOOTH_BITMAP $TestImage "$PLUGINSDIR\checker.bmp" 256 192 $TestBitmap
    IntOp $Replacements $Replacements + 1
  ${Loop}
  System::Call 'kernel32::GetCurrentProcess()p.r0'
  System::Call 'user32::GetGuiResources(pr0,i0)i.r1'
  !insertmacro EXPECT_EQ $1 $GdiBefore "GDI count after 64 replacements"
  FileWrite $TestLog "GDI objects: $GdiBefore before, $1 after 64 replacements$\r$\n"

  StrCpy $SavedBitmap $TestBitmap
  !insertmacro TA_SMOOTH_BITMAP $TestImage "$PLUGINSDIR\missing.bmp" 256 192 $TestBitmap
  !insertmacro EXPECT_EQ $TestBitmap $SavedBitmap "missing source retains ownership handle"
  SendMessage $TestImage ${STM_GETIMAGE} ${IMAGE_BITMAP} 0 $0
  !insertmacro EXPECT_EQ $0 $SavedBitmap "missing source retains displayed bitmap"
  Call CheckBitmap

  SendMessage $TestImage ${STM_SETIMAGE} ${IMAGE_BITMAP} 0 $0
  System::Call 'gdi32::DeleteObject(pr0)'
  System::Call 'user32::DestroyWindow(p$TestDialog)'
  ${If} $Failures == 0
    FileWrite $TestLog "PASS: filtered pixels, dimensions, aspect fit, GDI lifetime and fallback$\r$\n"
  ${Else}
    FileWrite $TestLog "FAIL: $Failures assertions$\r$\n"
  ${EndIf}
  FileClose $TestLog
  SetErrorLevel $Failures
  Quit
FunctionEnd

Function CheckBitmap
  System::Alloc ${TEST_BITMAP_SIZE}
  Pop $0
  System::Call 'gdi32::GetObject(p$TestBitmap,i${TEST_BITMAP_SIZE},pr0)i.r3'
  !insertmacro EXPECT_EQ $3 ${TEST_BITMAP_SIZE} "read output BITMAP"
  System::Call '*$0(i,i.r1,i.r2)'
  System::Free $0
  !insertmacro EXPECT_EQ $1 $ExpectedWidth "output bitmap width"
  !insertmacro EXPECT_EQ $2 $ExpectedHeight "output bitmap height"
  System::Call '*(i0,i0,i0,i0)p.r0'
  System::Call 'user32::GetClientRect(p$TestImage,pr0)i.r3'
  !insertmacro EXPECT_EQ $3 1 "read control rectangle"
  System::Call '*$0(i,i,i.r1,i.r2)'
  System::Free $0
  !insertmacro EXPECT_EQ $1 $ExpectedWidth "control width"
  !insertmacro EXPECT_EQ $2 $ExpectedHeight "control height"

  System::Call 'gdi32::CreateCompatibleDC(p0)p.r0'
  StrCpy $TestDC $0
  System::Call 'gdi32::SelectObject(p$TestDC,p$TestBitmap)p.r0'
  StrCpy $PreviousObject $0
  StrCpy $SampleY 0
  ${DoWhile} $SampleY < $ExpectedHeight
    StrCpy $SampleX 0
    ${DoWhile} $SampleX < $ExpectedWidth
      System::Call 'gdi32::GetPixel(p$TestDC,i$SampleX,i$SampleY)i.r0'
      IntOp $1 $0 & 0xff
      IntOp $2 $0 >> 8
      IntOp $2 $2 & 0xff
      IntOp $3 $0 >> 16
      IntOp $3 $3 & 0xff
      ; The source is alternating black/white pixels. Point sampling gives 0 or 255;
      ; averaging gives mid-gray. Allow a little rounding/dithering across display modes.
      ${If} $SampleX < $ContentLeft
      ${OrIf} $SampleX >= $ContentRight
      ${OrIf} $SampleY < $ContentTop
      ${OrIf} $SampleY >= $ContentBottom
        ${If} $0 <> 0x1A1511
          FileWrite $TestLog "FAIL: letterbox color at $SampleX,$SampleY = $0$\r$\n"
          IntOp $Failures $Failures + 1
          StrCpy $SampleY $ExpectedHeight
          ${Break}
        ${EndIf}
      ${Else}
        ${If} $1 < 112
        ${OrIf} $1 > 144
        ${OrIf} $2 != $1
        ${OrIf} $3 != $1
          FileWrite $TestLog "FAIL: unfiltered pixel at $SampleX,$SampleY = $0$\r$\n"
          IntOp $Failures $Failures + 1
          StrCpy $SampleY $ExpectedHeight
          ${Break}
        ${EndIf}
      ${EndIf}
      IntOp $SampleX $SampleX + 1
    ${Loop}
    IntOp $SampleY $SampleY + 1
  ${Loop}
  System::Call 'gdi32::SelectObject(p$TestDC,p$PreviousObject)'
  System::Call 'gdi32::DeleteDC(p$TestDC)'
FunctionEnd

Section
  ; Unreachable: RunArtworkTests quits before any page is shown or section executes.
SectionEnd
