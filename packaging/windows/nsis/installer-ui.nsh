; NeriPlayer 安装器界面：欢迎页、进度页、完成页与卸载页，以及四种语言的文案。
; 由 installer.nsi 在加载语言之后 !include（tauri.conf.json 的 installerHooks 指向本文件）。
;
; 版式：整窗白底；欢迎页与完成页画在 modern.exe 的 1044 区域（覆盖标题栏以下、按钮栏以上的整块），
; 进度页沿用标题栏 + 进度条。尺寸一律用对话框单位（u），随系统缩放与字体变化。

!define /ifndef SS_CENTER 0x00000001
!define /ifndef SS_NOPREFIX 0x00000080
!define /ifndef SS_ENDELLIPSIS 0x00004000
!define /ifndef SS_PATHELLIPSIS 0x00008000
!define /ifndef IMAGE_ICON 1

!define NERI_BG "FFFFFF"
!define NERI_TEXT "1B1B1F"
!define NERI_MUTED "5F6368"
!define NERI_ACCENT "1F4FB8"
!define NERI_WARN "B3261E"

Var DesktopShortcutState
Var LaunchAfterInstallState
Var DeleteAppDataState
Var NeriFontTitle
Var NeriFontHeader
Var NeriFontSmall
Var NeriPage
Var NeriPathLabel
Var NeriCheckbox

; ---------------------------------------------------------------------------
; 字体：中日文界面换成各自的系统 UI 字体，避免 Segoe UI 回退出的宋体
; ---------------------------------------------------------------------------
!ifdef LANG_ENGLISH
  SetFont /LANG=${LANG_ENGLISH} "Segoe UI" 9
!endif
!ifdef LANG_SIMPCHINESE
  SetFont /LANG=${LANG_SIMPCHINESE} "Microsoft YaHei UI" 9
!endif
!ifdef LANG_TRADCHINESE
  SetFont /LANG=${LANG_TRADCHINESE} "Microsoft JhengHei UI" 9
!endif
!ifdef LANG_JAPANESE
  SetFont /LANG=${LANG_JAPANESE} "Yu Gothic UI" 9
!endif

; ---------------------------------------------------------------------------
; 文案
; ---------------------------------------------------------------------------
!ifdef LANG_ENGLISH
  LangString neriFontFace ${LANG_ENGLISH} "Segoe UI"
  LangString neriCaption ${LANG_ENGLISH} "Install ${PRODUCTNAME}"
  LangString neriUninstallCaption ${LANG_ENGLISH} "Uninstall ${PRODUCTNAME}"
  LangString neriTagline ${LANG_ENGLISH} "Version ${VERSION}  ·  Your music, all in one place"
  LangString neriStatusFresh ${LANG_ENGLISH} "Click Install and you're done. It usually takes less than a minute."
  LangString neriStatusSame ${LANG_ENGLISH} "${PRODUCTNAME} ${VERSION} is already installed. Reinstall it to fix problems; your playlists and settings are kept."
  LangString neriStatusUpgrade ${LANG_ENGLISH} "This updates ${PRODUCTNAME} from $ExistingVersion to ${VERSION}. Your playlists and settings are kept."
  LangString neriStatusUpgradeUnknown ${LANG_ENGLISH} "This updates the installed ${PRODUCTNAME} to ${VERSION}. Your playlists and settings are kept."
  LangString neriStatusDowngrade ${LANG_ENGLISH} "A newer version ($ExistingVersion) is installed. Continuing replaces it with the older ${VERSION}."
  LangString neriStatusDowngradeBlocked ${LANG_ENGLISH} "A newer version ($ExistingVersion) is installed. This installer can't replace it with an older one."
  LangString neriStatusMsi ${LANG_ENGLISH} "The copy installed with the MSI package is removed first. Your playlists and settings are kept."
  LangString neriInstallTo ${LANG_ENGLISH} "Install location"
  LangString neriChange ${LANG_ENGLISH} "Change…"
  LangString neriChooseFolder ${LANG_ENGLISH} "Choose where to install ${PRODUCTNAME}"
  LangString neriDesktopShortcut ${LANG_ENGLISH} "Add a shortcut to the desktop"
  LangString neriInstallButton ${LANG_ENGLISH} "Install"
  LangString neriUpgradeButton ${LANG_ENGLISH} "Update"
  LangString neriReinstallButton ${LANG_ENGLISH} "Reinstall"
  LangString neriInstallingTitle ${LANG_ENGLISH} "Installing ${PRODUCTNAME}"
  LangString neriInstallingSubtitle ${LANG_ENGLISH} "Hang tight, this only takes a moment."
  LangString neriDoneTitle ${LANG_ENGLISH} "All set"
  LangString neriDoneText ${LANG_ENGLISH} "${PRODUCTNAME} is ready. You'll find it in the Start menu."
  LangString neriLaunchNow ${LANG_ENGLISH} "Open ${PRODUCTNAME} now"
  LangString neriFinishButton ${LANG_ENGLISH} "Finish"
  LangString neriCancelled ${LANG_ENGLISH} "Cancelled."
  LangString neriRemovingOld ${LANG_ENGLISH} "Removing the previous version…"
  LangString neriRemovingMsi ${LANG_ENGLISH} "Removing the copy installed with the MSI package…"
  LangString neriMoveFailed ${LANG_ENGLISH} "Couldn't remove ${PRODUCTNAME} from its old location ($PreviousInstallDir), so the installation was stopped. Close ${PRODUCTNAME} or uninstall it from Settings, then try again."
  LangString neriUninstallTitle ${LANG_ENGLISH} "Uninstall ${PRODUCTNAME}"
  LangString neriUninstallText ${LANG_ENGLISH} "${PRODUCTNAME} will be removed from this PC. Your playlists, settings and other data are kept unless you choose otherwise, so a later reinstall picks up where you left off."
  LangString neriDeleteData ${LANG_ENGLISH} "Also delete my data (playlists, settings and cache)"
  LangString neriUninstallButton ${LANG_ENGLISH} "Uninstall"
  LangString neriUninstallingTitle ${LANG_ENGLISH} "Uninstalling ${PRODUCTNAME}"
  LangString neriUninstallingSubtitle ${LANG_ENGLISH} "Just a moment."
  LangString neriUninstalledTitle ${LANG_ENGLISH} "Uninstalled"
  LangString neriUninstalledText ${LANG_ENGLISH} "${PRODUCTNAME} has been removed from this PC. Thanks for listening!"
!endif

!ifdef LANG_SIMPCHINESE
  LangString neriFontFace ${LANG_SIMPCHINESE} "Microsoft YaHei UI"
  LangString neriCaption ${LANG_SIMPCHINESE} "安装 ${PRODUCTNAME}"
  LangString neriUninstallCaption ${LANG_SIMPCHINESE} "卸载 ${PRODUCTNAME}"
  LangString neriTagline ${LANG_SIMPCHINESE} "版本 ${VERSION}  ·  把你喜欢的音乐聚到一处"
  LangString neriStatusFresh ${LANG_SIMPCHINESE} "点击「立即安装」就好，通常不到一分钟。"
  LangString neriStatusSame ${LANG_SIMPCHINESE} "这台电脑上已经装有 ${PRODUCTNAME} ${VERSION}。重新安装可以修复问题，歌单和设置都会保留。"
  LangString neriStatusUpgrade ${LANG_SIMPCHINESE} "将把 ${PRODUCTNAME} 从 $ExistingVersion 升级到 ${VERSION}，歌单和设置都会保留。"
  LangString neriStatusUpgradeUnknown ${LANG_SIMPCHINESE} "将把已安装的 ${PRODUCTNAME} 升级到 ${VERSION}，歌单和设置都会保留。"
  LangString neriStatusDowngrade ${LANG_SIMPCHINESE} "这台电脑上已有更新的版本（$ExistingVersion），继续会换成较旧的 ${VERSION}。"
  LangString neriStatusDowngradeBlocked ${LANG_SIMPCHINESE} "这台电脑上已有更新的版本（$ExistingVersion），此安装程序不能换成旧版本。"
  LangString neriStatusMsi ${LANG_SIMPCHINESE} "会先移除通过 MSI 安装的旧版本，歌单和设置都会保留。"
  LangString neriInstallTo ${LANG_SIMPCHINESE} "安装位置"
  LangString neriChange ${LANG_SIMPCHINESE} "更改…"
  LangString neriChooseFolder ${LANG_SIMPCHINESE} "选择 ${PRODUCTNAME} 的安装位置"
  LangString neriDesktopShortcut ${LANG_SIMPCHINESE} "在桌面创建快捷方式"
  LangString neriInstallButton ${LANG_SIMPCHINESE} "立即安装"
  LangString neriUpgradeButton ${LANG_SIMPCHINESE} "立即升级"
  LangString neriReinstallButton ${LANG_SIMPCHINESE} "重新安装"
  LangString neriInstallingTitle ${LANG_SIMPCHINESE} "正在安装 ${PRODUCTNAME}"
  LangString neriInstallingSubtitle ${LANG_SIMPCHINESE} "请稍候，马上就好。"
  LangString neriDoneTitle ${LANG_SIMPCHINESE} "安装完成"
  LangString neriDoneText ${LANG_SIMPCHINESE} "${PRODUCTNAME} 已经准备好了，可以在开始菜单里找到它。"
  LangString neriLaunchNow ${LANG_SIMPCHINESE} "立即打开 ${PRODUCTNAME}"
  LangString neriFinishButton ${LANG_SIMPCHINESE} "完成"
  LangString neriCancelled ${LANG_SIMPCHINESE} "已取消。"
  LangString neriRemovingOld ${LANG_SIMPCHINESE} "正在移除旧版本…"
  LangString neriRemovingMsi ${LANG_SIMPCHINESE} "正在移除通过 MSI 安装的旧版本…"
  LangString neriMoveFailed ${LANG_SIMPCHINESE} "无法移除旧位置（$PreviousInstallDir）的 ${PRODUCTNAME}，已停止安装。请先关闭 ${PRODUCTNAME}，或在系统设置里卸载后再试。"
  LangString neriUninstallTitle ${LANG_SIMPCHINESE} "卸载 ${PRODUCTNAME}"
  LangString neriUninstallText ${LANG_SIMPCHINESE} "${PRODUCTNAME} 将从这台电脑上移除。歌单、设置等数据默认会保留，以后重新安装可以接着用。"
  LangString neriDeleteData ${LANG_SIMPCHINESE} "同时删除我的数据（歌单、设置和缓存）"
  LangString neriUninstallButton ${LANG_SIMPCHINESE} "卸载"
  LangString neriUninstallingTitle ${LANG_SIMPCHINESE} "正在卸载 ${PRODUCTNAME}"
  LangString neriUninstallingSubtitle ${LANG_SIMPCHINESE} "请稍候。"
  LangString neriUninstalledTitle ${LANG_SIMPCHINESE} "已卸载"
  LangString neriUninstalledText ${LANG_SIMPCHINESE} "${PRODUCTNAME} 已从这台电脑上移除，感谢使用！"
!endif

!ifdef LANG_TRADCHINESE
  LangString neriFontFace ${LANG_TRADCHINESE} "Microsoft JhengHei UI"
  LangString neriCaption ${LANG_TRADCHINESE} "安裝 ${PRODUCTNAME}"
  LangString neriUninstallCaption ${LANG_TRADCHINESE} "解除安裝 ${PRODUCTNAME}"
  LangString neriTagline ${LANG_TRADCHINESE} "版本 ${VERSION}  ·  把你喜歡的音樂聚在一起"
  LangString neriStatusFresh ${LANG_TRADCHINESE} "點擊「立即安裝」就好，通常不到一分鐘。"
  LangString neriStatusSame ${LANG_TRADCHINESE} "這台電腦上已經裝有 ${PRODUCTNAME} ${VERSION}。重新安裝可以修復問題，歌單和設定都會保留。"
  LangString neriStatusUpgrade ${LANG_TRADCHINESE} "將把 ${PRODUCTNAME} 從 $ExistingVersion 升級到 ${VERSION}，歌單和設定都會保留。"
  LangString neriStatusUpgradeUnknown ${LANG_TRADCHINESE} "將把已安裝的 ${PRODUCTNAME} 升級到 ${VERSION}，歌單和設定都會保留。"
  LangString neriStatusDowngrade ${LANG_TRADCHINESE} "這台電腦上已有較新的版本（$ExistingVersion），繼續會換成較舊的 ${VERSION}。"
  LangString neriStatusDowngradeBlocked ${LANG_TRADCHINESE} "這台電腦上已有較新的版本（$ExistingVersion），此安裝程式無法換成舊版本。"
  LangString neriStatusMsi ${LANG_TRADCHINESE} "會先移除透過 MSI 安裝的舊版本，歌單和設定都會保留。"
  LangString neriInstallTo ${LANG_TRADCHINESE} "安裝位置"
  LangString neriChange ${LANG_TRADCHINESE} "變更…"
  LangString neriChooseFolder ${LANG_TRADCHINESE} "選擇 ${PRODUCTNAME} 的安裝位置"
  LangString neriDesktopShortcut ${LANG_TRADCHINESE} "在桌面建立捷徑"
  LangString neriInstallButton ${LANG_TRADCHINESE} "立即安裝"
  LangString neriUpgradeButton ${LANG_TRADCHINESE} "立即升級"
  LangString neriReinstallButton ${LANG_TRADCHINESE} "重新安裝"
  LangString neriInstallingTitle ${LANG_TRADCHINESE} "正在安裝 ${PRODUCTNAME}"
  LangString neriInstallingSubtitle ${LANG_TRADCHINESE} "請稍候，馬上就好。"
  LangString neriDoneTitle ${LANG_TRADCHINESE} "安裝完成"
  LangString neriDoneText ${LANG_TRADCHINESE} "${PRODUCTNAME} 已經準備好了，可以在開始功能表裡找到它。"
  LangString neriLaunchNow ${LANG_TRADCHINESE} "立即開啟 ${PRODUCTNAME}"
  LangString neriFinishButton ${LANG_TRADCHINESE} "完成"
  LangString neriCancelled ${LANG_TRADCHINESE} "已取消。"
  LangString neriRemovingOld ${LANG_TRADCHINESE} "正在移除舊版本…"
  LangString neriRemovingMsi ${LANG_TRADCHINESE} "正在移除透過 MSI 安裝的舊版本…"
  LangString neriMoveFailed ${LANG_TRADCHINESE} "無法移除舊位置（$PreviousInstallDir）的 ${PRODUCTNAME}，已停止安裝。請先關閉 ${PRODUCTNAME}，或在系統設定裡解除安裝後再試。"
  LangString neriUninstallTitle ${LANG_TRADCHINESE} "解除安裝 ${PRODUCTNAME}"
  LangString neriUninstallText ${LANG_TRADCHINESE} "${PRODUCTNAME} 將從這台電腦上移除。歌單、設定等資料預設會保留，日後重新安裝可以接著使用。"
  LangString neriDeleteData ${LANG_TRADCHINESE} "同時刪除我的資料（歌單、設定和快取）"
  LangString neriUninstallButton ${LANG_TRADCHINESE} "解除安裝"
  LangString neriUninstallingTitle ${LANG_TRADCHINESE} "正在解除安裝 ${PRODUCTNAME}"
  LangString neriUninstallingSubtitle ${LANG_TRADCHINESE} "請稍候。"
  LangString neriUninstalledTitle ${LANG_TRADCHINESE} "已解除安裝"
  LangString neriUninstalledText ${LANG_TRADCHINESE} "${PRODUCTNAME} 已從這台電腦上移除，感謝使用！"
!endif

!ifdef LANG_JAPANESE
  LangString neriFontFace ${LANG_JAPANESE} "Yu Gothic UI"
  LangString neriCaption ${LANG_JAPANESE} "${PRODUCTNAME} のインストール"
  LangString neriUninstallCaption ${LANG_JAPANESE} "${PRODUCTNAME} のアンインストール"
  LangString neriTagline ${LANG_JAPANESE} "バージョン ${VERSION}  ·  好きな音楽をひとつの場所に"
  LangString neriStatusFresh ${LANG_JAPANESE} "「インストール」を押すだけです。通常 1 分もかかりません。"
  LangString neriStatusSame ${LANG_JAPANESE} "${PRODUCTNAME} ${VERSION} はすでにインストールされています。再インストールで問題を修復できます。プレイリストと設定はそのまま残ります。"
  LangString neriStatusUpgrade ${LANG_JAPANESE} "${PRODUCTNAME} を $ExistingVersion から ${VERSION} に更新します。プレイリストと設定はそのまま残ります。"
  LangString neriStatusUpgradeUnknown ${LANG_JAPANESE} "インストール済みの ${PRODUCTNAME} を ${VERSION} に更新します。プレイリストと設定はそのまま残ります。"
  LangString neriStatusDowngrade ${LANG_JAPANESE} "新しいバージョン（$ExistingVersion）がインストールされています。続けると古い ${VERSION} に置き換わります。"
  LangString neriStatusDowngradeBlocked ${LANG_JAPANESE} "新しいバージョン（$ExistingVersion）がインストールされています。このインストーラーでは古いバージョンに戻せません。"
  LangString neriStatusMsi ${LANG_JAPANESE} "MSI パッケージでインストールされた版を先に削除します。プレイリストと設定はそのまま残ります。"
  LangString neriInstallTo ${LANG_JAPANESE} "インストール先"
  LangString neriChange ${LANG_JAPANESE} "変更…"
  LangString neriChooseFolder ${LANG_JAPANESE} "${PRODUCTNAME} のインストール先を選択"
  LangString neriDesktopShortcut ${LANG_JAPANESE} "デスクトップにショートカットを作成"
  LangString neriInstallButton ${LANG_JAPANESE} "インストール"
  LangString neriUpgradeButton ${LANG_JAPANESE} "更新"
  LangString neriReinstallButton ${LANG_JAPANESE} "再インストール"
  LangString neriInstallingTitle ${LANG_JAPANESE} "${PRODUCTNAME} をインストールしています"
  LangString neriInstallingSubtitle ${LANG_JAPANESE} "少々お待ちください。"
  LangString neriDoneTitle ${LANG_JAPANESE} "インストール完了"
  LangString neriDoneText ${LANG_JAPANESE} "${PRODUCTNAME} の準備ができました。スタートメニューから起動できます。"
  LangString neriLaunchNow ${LANG_JAPANESE} "今すぐ ${PRODUCTNAME} を開く"
  LangString neriFinishButton ${LANG_JAPANESE} "完了"
  LangString neriCancelled ${LANG_JAPANESE} "キャンセルしました。"
  LangString neriRemovingOld ${LANG_JAPANESE} "以前のバージョンを削除しています…"
  LangString neriRemovingMsi ${LANG_JAPANESE} "MSI パッケージでインストールされた版を削除しています…"
  LangString neriMoveFailed ${LANG_JAPANESE} "以前の場所（$PreviousInstallDir）の ${PRODUCTNAME} を削除できなかったため、インストールを中止しました。${PRODUCTNAME} を閉じるか、設定からアンインストールしてから再試行してください。"
  LangString neriUninstallTitle ${LANG_JAPANESE} "${PRODUCTNAME} のアンインストール"
  LangString neriUninstallText ${LANG_JAPANESE} "${PRODUCTNAME} をこの PC から削除します。プレイリストや設定などのデータは残るので、あとで再インストールしても続きから使えます。"
  LangString neriDeleteData ${LANG_JAPANESE} "データも削除する（プレイリスト、設定、キャッシュ）"
  LangString neriUninstallButton ${LANG_JAPANESE} "アンインストール"
  LangString neriUninstallingTitle ${LANG_JAPANESE} "${PRODUCTNAME} をアンインストールしています"
  LangString neriUninstallingSubtitle ${LANG_JAPANESE} "少々お待ちください。"
  LangString neriUninstalledTitle ${LANG_JAPANESE} "アンインストール完了"
  LangString neriUninstalledText ${LANG_JAPANESE} "${PRODUCTNAME} をこの PC から削除しました。ご利用ありがとうございました！"
!endif

; ---------------------------------------------------------------------------
; 公共宏
; ---------------------------------------------------------------------------

; 在整块内容区（1044）上建一页，白底，隐藏「上一步」
!macro NeriCreatePage
  nsDialogs::Create 1044
  Pop $NeriPage
  ${If} $NeriPage == error
    Abort
  ${EndIf}
  SetCtlColors $NeriPage "" "${NERI_BG}"
  GetDlgItem $0 $HWNDPARENT 3
  ShowWindow $0 ${SW_HIDE}
!macroend

; 居中的应用图标：按控件的实际像素尺寸从安装程序自身资源取图，高 DPI 下也清晰
!macro NeriAppIcon
  ${NSD_CreateIcon} 145u 22u 41u 38u ""
  Pop $1
  ${NSD_AddStyle} $1 0x00000200 ; SS_CENTERIMAGE：控件不随图标尺寸改变
  System::Call '*(i, i, i, i) p .r2'
  System::Call 'user32::GetClientRect(p r1, p r2)'
  System::Call '*$2(i, i, i .r3, i .r4)'
  System::Free $2
  ${If} $4 < $3
    StrCpy $3 $4
  ${EndIf}
  System::Call 'kernel32::GetModuleHandle(p 0) p .r5'
  System::Call 'user32::LoadImage(p r5, p 103, i ${IMAGE_ICON}, i r3, i r3, i 0) p .r6'
  SendMessage $1 ${STM_SETICON} $6 0
  SetCtlColors $1 "" "${NERI_BG}"
!macroend

; 居中的一行文字；颜色为 RRGGBB，字体句柄为 0 时沿用对话框字体
!macro NeriCenterLabel Y H TEXT COLOR FONT
  ${NSD_CreateLabel} 16u ${Y} 299u ${H} "${TEXT}"
  Pop $1
  ${NSD_AddStyle} $1 ${SS_CENTER}|${SS_NOPREFIX}
  SetCtlColors $1 "${COLOR}" "${NERI_BG}"
  ${If} ${FONT} != 0
    SendMessage $1 ${WM_SETFONT} ${FONT} 1
  ${EndIf}
!macroend

!macro NeriCheckbox Y TEXT STATE
  ${NSD_CreateCheckbox} 24u ${Y} 283u 12u "${TEXT}"
  Pop $NeriCheckbox
  SetCtlColors $NeriCheckbox "${NERI_TEXT}" "${NERI_BG}"
  ${NSD_SetState} $NeriCheckbox ${STATE}
!macroend

!macro NeriButtonText ID TEXT
  GetDlgItem $0 $HWNDPARENT ${ID}
  SendMessage $0 ${WM_SETTEXT} 0 "STR:${TEXT}"
!macroend

!macro NeriHideButton ID
  GetDlgItem $0 $HWNDPARENT ${ID}
  ShowWindow $0 ${SW_HIDE}
!macroend

; 进度页沿用标题栏：写入标题与副标题，内容区刷成白底
!macro NeriProgressHeader TITLE SUBTITLE
  GetDlgItem $0 $HWNDPARENT 1037
  SendMessage $0 ${WM_SETFONT} $NeriFontHeader 1
  SendMessage $0 ${WM_SETTEXT} 0 "STR:${TITLE}"
  GetDlgItem $0 $HWNDPARENT 1038
  SendMessage $0 ${WM_SETTEXT} 0 "STR:${SUBTITLE}"
  FindWindow $1 "#32770" "" $HWNDPARENT
  SetCtlColors $1 "" "${NERI_BG}"
  GetDlgItem $0 $1 1006
  SetCtlColors $0 "${NERI_MUTED}" "${NERI_BG}"
  GetDlgItem $0 $1 1004
  SetCtlColors $0 "" "${NERI_BG}"
!macroend

; 字体与整窗配色；安装程序和卸载程序各需一份（卸载程序的函数必须以 un. 开头）
!macro NeriDefineWindowSetup PREFIX
  Function ${PREFIX}NeriInitFonts
    CreateFont $NeriFontTitle "$(neriFontFace)" 20 700
    CreateFont $NeriFontHeader "$(neriFontFace)" 10 700
    CreateFont $NeriFontSmall "$(neriFontFace)" 8 400

    SetCtlColors $HWNDPARENT "" "${NERI_BG}"
    ; 标题栏背景、标题、副标题、右上角图标
    GetDlgItem $0 $HWNDPARENT 1034
    SetCtlColors $0 "" "${NERI_BG}"
    GetDlgItem $0 $HWNDPARENT 1037
    SetCtlColors $0 "${NERI_TEXT}" "${NERI_BG}"
    GetDlgItem $0 $HWNDPARENT 1038
    SetCtlColors $0 "${NERI_MUTED}" "${NERI_BG}"
    GetDlgItem $0 $HWNDPARENT 1039
    SetCtlColors $0 "" "${NERI_BG}"
    ; 品牌文字留空，只保留按钮上方的分隔线
    GetDlgItem $0 $HWNDPARENT 1028
    ShowWindow $0 ${SW_HIDE}
    GetDlgItem $0 $HWNDPARENT 1256
    ShowWindow $0 ${SW_HIDE}
  FunctionEnd
!macroend
!insertmacro NeriDefineWindowSetup ""
!insertmacro NeriDefineWindowSetup "un."

; ---------------------------------------------------------------------------
; 安装：欢迎页
; ---------------------------------------------------------------------------
Function WelcomePageCreate
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
  ; 升级时默认沿用用户原来的选择：之前删掉了桌面快捷方式就不再自动建回来
  ${If} $ExistingKind > 0
  ${AndIfNot} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
    StrCpy $DesktopShortcutState ${BST_UNCHECKED}
  ${EndIf}

  !insertmacro NeriCreatePage
  !insertmacro NeriAppIcon
  !insertmacro NeriCenterLabel 66u 26u "${PRODUCTNAME}" "${NERI_TEXT}" $NeriFontTitle
  !insertmacro NeriCenterLabel 92u 10u "$(neriTagline)" "${NERI_MUTED}" 0

  ; 一句话说明这次会发生什么
  StrCpy $7 "${NERI_MUTED}"
  ${If} $WixMode = 1
    StrCpy $8 "$(neriStatusMsi)"
  ${ElseIf} $ExistingKind = 1
    StrCpy $8 "$(neriStatusSame)"
  ${ElseIf} $ExistingKind = 2
    ${If} $ExistingVersion == ""
      StrCpy $8 "$(neriStatusUpgradeUnknown)"
    ${Else}
      StrCpy $8 "$(neriStatusUpgrade)"
    ${EndIf}
  ${ElseIf} $ExistingKind = 3
    StrCpy $7 "${NERI_WARN}"
    !if "${ALLOWDOWNGRADES}" == "false"
      StrCpy $8 "$(neriStatusDowngradeBlocked)"
    !else
      StrCpy $8 "$(neriStatusDowngrade)"
    !endif
  ${Else}
    StrCpy $8 "$(neriStatusFresh)"
  ${EndIf}
  !insertmacro NeriCenterLabel 110u 22u "$8" "$7" 0

  ; 安装位置：已安装时默认沿用原位置，也可以换；换了位置会先移除旧位置那份（见 RemovePrevious）
  ${NSD_CreateLabel} 24u 138u 200u 9u "$(neriInstallTo)"
  Pop $1
  SetCtlColors $1 "${NERI_MUTED}" "${NERI_BG}"
  SendMessage $1 ${WM_SETFONT} $NeriFontSmall 1
  ${NSD_CreateLabel} 24u 149u 228u 11u "$INSTDIR"
  Pop $NeriPathLabel
  ${NSD_AddStyle} $NeriPathLabel ${SS_PATHELLIPSIS}|${SS_NOPREFIX}
  SetCtlColors $NeriPathLabel "${NERI_TEXT}" "${NERI_BG}"
  ${NSD_CreateLink} 256u 149u 51u 11u "$(neriChange)"
  Pop $1
  ${NSD_AddStyle} $1 ${SS_NOPREFIX}
  SetCtlColors $1 "${NERI_ACCENT}" "${NERI_BG}"
  ${NSD_OnClick} $1 WelcomeChangeDirectory

  !insertmacro NeriCheckbox 167u "$(neriDesktopShortcut)" $DesktopShortcutState

  ${If} $ExistingKind = 1
    !insertmacro NeriButtonText 1 "$(neriReinstallButton)"
  ${ElseIf} $ExistingKind = 2
    !insertmacro NeriButtonText 1 "$(neriUpgradeButton)"
  ${Else}
    !insertmacro NeriButtonText 1 "$(neriInstallButton)"
  ${EndIf}
  !if "${ALLOWDOWNGRADES}" == "false"
    ${If} $ExistingKind = 3
      GetDlgItem $0 $HWNDPARENT 1
      EnableWindow $0 0
    ${EndIf}
  !endif

  nsDialogs::Show
FunctionEnd

Function WelcomePageLeave
  ${NSD_GetState} $NeriCheckbox $DesktopShortcutState
FunctionEnd

Function WelcomeChangeDirectory
  Pop $0
  nsDialogs::SelectFolderDialog "$(neriChooseFolder)" "$INSTDIR"
  Pop $0
  ${If} $0 == error
  ${OrIf} $0 == ""
    Return
  ${EndIf}
  ; 选到盘符根目录或任意已有文件夹时，总是装进其中的产品子目录，卸载时不会波及同级文件
  StrCpy $1 $0 1 -1
  ${If} $1 == "\"
    StrCpy $0 $0 -1
  ${EndIf}
  ${GetFileName} $0 $1
  ${If} $1 != "${PRODUCTNAME}"
    StrCpy $0 "$0\${PRODUCTNAME}"
  ${EndIf}
  StrCpy $INSTDIR $0
  ${NSD_SetText} $NeriPathLabel $INSTDIR
FunctionEnd

; ---------------------------------------------------------------------------
; 安装：进度页与完成页
; ---------------------------------------------------------------------------
Function InstFilesShow
  !insertmacro NeriProgressHeader "$(neriInstallingTitle)" "$(neriInstallingSubtitle)"
FunctionEnd

Function FinishPageCreate
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
  !insertmacro NeriCreatePage
  !insertmacro NeriHideButton 2
  !insertmacro NeriAppIcon
  !insertmacro NeriCenterLabel 66u 26u "$(neriDoneTitle)" "${NERI_TEXT}" $NeriFontTitle
  !insertmacro NeriCenterLabel 94u 22u "$(neriDoneText)" "${NERI_MUTED}" 0
  !insertmacro NeriCheckbox 167u "$(neriLaunchNow)" $LaunchAfterInstallState
  !insertmacro NeriButtonText 1 "$(neriFinishButton)"
  nsDialogs::Show
FunctionEnd

Function FinishPageLeave
  ${NSD_GetState} $NeriCheckbox $LaunchAfterInstallState
  ${If} $LaunchAfterInstallState == ${BST_CHECKED}
    nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------------------
; 卸载
; ---------------------------------------------------------------------------
Function un.ConfirmPageCreate
  StrCpy $DeleteAppDataState ${BST_UNCHECKED}
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}
  !insertmacro NeriCreatePage
  !insertmacro NeriAppIcon
  !insertmacro NeriCenterLabel 66u 26u "$(neriUninstallTitle)" "${NERI_TEXT}" $NeriFontTitle
  !insertmacro NeriCenterLabel 94u 34u "$(neriUninstallText)" "${NERI_MUTED}" 0
  !insertmacro NeriCheckbox 167u "$(neriDeleteData)" $DeleteAppDataState
  !insertmacro NeriButtonText 1 "$(neriUninstallButton)"
  nsDialogs::Show
FunctionEnd

Function un.ConfirmPageLeave
  ${NSD_GetState} $NeriCheckbox $DeleteAppDataState
FunctionEnd

Function un.InstFilesShow
  !insertmacro NeriProgressHeader "$(neriUninstallingTitle)" "$(neriUninstallingSubtitle)"
FunctionEnd

Function un.DonePageCreate
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}
  !insertmacro NeriCreatePage
  !insertmacro NeriHideButton 2
  !insertmacro NeriAppIcon
  !insertmacro NeriCenterLabel 66u 26u "$(neriUninstalledTitle)" "${NERI_TEXT}" $NeriFontTitle
  !insertmacro NeriCenterLabel 94u 22u "$(neriUninstalledText)" "${NERI_MUTED}" 0
  !insertmacro NeriButtonText 1 "$(neriFinishButton)"
  nsDialogs::Show
FunctionEnd

; 应用内增量更新留下的让位文件与暂存目录；ffmpeg 目录整个属于本应用，更新可能往里加过新文件
Function un.RemoveUpdateLeftovers
  RMDir /r "$INSTDIR\.neri-update"
  ; 让位文件只删更新器记录过的（.neri-update-asides，相对安装目录一行一个），不按文件名猜；
  ; 记录被改坏时，带 ..、盘符或以斜杠开头的行一律跳过
  ClearErrors
  FileOpen $R0 "$INSTDIR\.neri-update-asides" r
  ${IfNot} ${Errors}
    ${Do}
      ClearErrors
      FileRead $R0 $R1
      ${If} ${Errors}
        ${ExitDo}
      ${EndIf}
      trim_line:
        StrCpy $R2 $R1 1 -1
        ${If} $R2 == "$\n"
        ${OrIf} $R2 == "$\r"
          StrCpy $R1 $R1 -1
          Goto trim_line
        ${EndIf}
      ${If} $R1 == ""
        ${Continue}
      ${EndIf}
      StrCpy $R2 $R1 1
      ${If} $R2 == "\"
      ${OrIf} $R2 == "/"
        ${Continue}
      ${EndIf}
      nsis_tauri_utils::StrReplace "$R1" ".." ""
      Pop $R2
      ${If} $R2 != $R1
        ${Continue}
      ${EndIf}
      nsis_tauri_utils::StrReplace "$R1" ":" ""
      Pop $R2
      ${If} $R2 != $R1
        ${Continue}
      ${EndIf}
      Delete "$INSTDIR\$R1"
    ${Loop}
    FileClose $R0
  ${EndIf}
  Delete "$INSTDIR\.neri-update-asides"
  RMDir /r "$INSTDIR\ffmpeg"
FunctionEnd
