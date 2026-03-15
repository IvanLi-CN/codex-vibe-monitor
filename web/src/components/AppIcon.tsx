import { Icon } from '@iconify/react'
import type { IconifyIcon } from '@iconify/types'
import type { ComponentProps } from 'react'
import accountDetailsOutlineIcon from '@iconify-icons/mdi/account-details-outline'
import alertCircleIcon from '@iconify-icons/mdi/alert-circle'
import alertCircleOutlineIcon from '@iconify-icons/mdi/alert-circle-outline'
import alertDecagramOutlineIcon from '@iconify-icons/mdi/alert-decagram-outline'
import alertOutlineIcon from '@iconify-icons/mdi/alert-outline'
import arrowDownBoldIcon from '@iconify-icons/mdi/arrow-down-bold'
import arrowLeftIcon from '@iconify-icons/mdi/arrow-left'
import arrowRightBoldIcon from '@iconify-icons/mdi/arrow-right-bold'
import arrowUpBoldIcon from '@iconify-icons/mdi/arrow-up-bold'
import autoFixIcon from '@iconify-icons/mdi/auto-fix'
import badgeAccountHorizontalOutlineIcon from '@iconify-icons/mdi/badge-account-horizontal-outline'
import checkIcon from '@iconify-icons/mdi/check'
import checkBoldIcon from '@iconify-icons/mdi/check-bold'
import checkCircleOutlineIcon from '@iconify-icons/mdi/check-circle-outline'
import checkDecagramOutlineIcon from '@iconify-icons/mdi/check-decagram-outline'
import chevronDownIcon from '@iconify-icons/mdi/chevron-down'
import chevronRightIcon from '@iconify-icons/mdi/chevron-right'
import chevronRightCircleIcon from '@iconify-icons/mdi/chevron-right-circle'
import chevronUpIcon from '@iconify-icons/mdi/chevron-up'
import closeIcon from '@iconify-icons/mdi/close'
import contentCopyIcon from '@iconify-icons/mdi/content-copy'
import contentSaveOutlineIcon from '@iconify-icons/mdi/content-save-outline'
import contentSavePlusOutlineIcon from '@iconify-icons/mdi/content-save-plus-outline'
import crownIcon from '@iconify-icons/mdi/crown'
import crownOutlineIcon from '@iconify-icons/mdi/crown-outline'
import databaseOutlineIcon from '@iconify-icons/mdi/database-outline'
import deleteOutlineIcon from '@iconify-icons/mdi/delete-outline'
import dotsHorizontalIcon from '@iconify-icons/mdi/dots-horizontal'
import earthIcon from '@iconify-icons/mdi/earth'
import fileDocumentEditOutlineIcon from '@iconify-icons/mdi/file-document-edit-outline'
import githubIcon from '@iconify-icons/mdi/github'
import helpCircleOutlineIcon from '@iconify-icons/mdi/help-circle-outline'
import informationOutlineIcon from '@iconify-icons/mdi/information-outline'
import keyChainVariantIcon from '@iconify-icons/mdi/key-chain-variant'
import keyOutlineIcon from '@iconify-icons/mdi/key-outline'
import lightningBoltIcon from '@iconify-icons/mdi/lightning-bolt'
import linkVariantOffIcon from '@iconify-icons/mdi/link-variant-off'
import linkVariantPlusIcon from '@iconify-icons/mdi/link-variant-plus'
import loadingIcon from '@iconify-icons/mdi/loading'
import loginVariantIcon from '@iconify-icons/mdi/login-variant'
import magnifyIcon from '@iconify-icons/mdi/magnify'
import noteTextOutlineIcon from '@iconify-icons/mdi/note-text-outline'
import pencilOutlineIcon from '@iconify-icons/mdi/pencil-outline'
import playlistPlusIcon from '@iconify-icons/mdi/playlist-plus'
import plusIcon from '@iconify-icons/mdi/plus'
import plusCircleOutlineIcon from '@iconify-icons/mdi/plus-circle-outline'
import refreshIcon from '@iconify-icons/mdi/refresh'
import refreshCircleIcon from '@iconify-icons/mdi/refresh-circle'
import serverNetworkOutlineIcon from '@iconify-icons/mdi/server-network-outline'
import shieldKeyOutlineIcon from '@iconify-icons/mdi/shield-key-outline'
import tagOutlineIcon from '@iconify-icons/mdi/tag-outline'
import tagPlusOutlineIcon from '@iconify-icons/mdi/tag-plus-outline'
import timerRefreshOutlineIcon from '@iconify-icons/mdi/timer-refresh-outline'
import trashCanOutlineIcon from '@iconify-icons/mdi/trash-can-outline'
import undoVariantIcon from '@iconify-icons/mdi/undo-variant'
import weatherNightIcon from '@iconify-icons/mdi/weather-night'
import whiteBalanceSunnyIcon from '@iconify-icons/mdi/white-balance-sunny'

const appIconRegistry = {
  'account-details-outline': accountDetailsOutlineIcon,
  'alert-circle': alertCircleIcon,
  'alert-circle-outline': alertCircleOutlineIcon,
  'alert-decagram-outline': alertDecagramOutlineIcon,
  'alert-outline': alertOutlineIcon,
  'arrow-down-bold': arrowDownBoldIcon,
  'arrow-left': arrowLeftIcon,
  'arrow-right-bold': arrowRightBoldIcon,
  'arrow-up-bold': arrowUpBoldIcon,
  'auto-fix': autoFixIcon,
  'badge-account-horizontal-outline': badgeAccountHorizontalOutlineIcon,
  'check': checkIcon,
  'check-bold': checkBoldIcon,
  'check-circle-outline': checkCircleOutlineIcon,
  'check-decagram-outline': checkDecagramOutlineIcon,
  'chevron-down': chevronDownIcon,
  'chevron-right': chevronRightIcon,
  'chevron-right-circle': chevronRightCircleIcon,
  'chevron-up': chevronUpIcon,
  'close': closeIcon,
  'content-copy': contentCopyIcon,
  'content-save-outline': contentSaveOutlineIcon,
  'content-save-plus-outline': contentSavePlusOutlineIcon,
  'crown': crownIcon,
  'crown-outline': crownOutlineIcon,
  'database-outline': databaseOutlineIcon,
  'delete-outline': deleteOutlineIcon,
  'dots-horizontal': dotsHorizontalIcon,
  'earth': earthIcon,
  'file-document-edit-outline': fileDocumentEditOutlineIcon,
  'github': githubIcon,
  'help-circle-outline': helpCircleOutlineIcon,
  'information-outline': informationOutlineIcon,
  'key-chain-variant': keyChainVariantIcon,
  'key-outline': keyOutlineIcon,
  'lightning-bolt': lightningBoltIcon,
  'link-variant-off': linkVariantOffIcon,
  'link-variant-plus': linkVariantPlusIcon,
  'loading': loadingIcon,
  'login-variant': loginVariantIcon,
  'magnify': magnifyIcon,
  'note-text-outline': noteTextOutlineIcon,
  'pencil-outline': pencilOutlineIcon,
  'playlist-plus': playlistPlusIcon,
  'plus': plusIcon,
  'plus-circle-outline': plusCircleOutlineIcon,
  'refresh': refreshIcon,
  'refresh-circle': refreshCircleIcon,
  'server-network-outline': serverNetworkOutlineIcon,
  'shield-key-outline': shieldKeyOutlineIcon,
  'tag-outline': tagOutlineIcon,
  'tag-plus-outline': tagPlusOutlineIcon,
  'timer-refresh-outline': timerRefreshOutlineIcon,
  'trash-can-outline': trashCanOutlineIcon,
  'undo-variant': undoVariantIcon,
  'weather-night': weatherNightIcon,
  'white-balance-sunny': whiteBalanceSunnyIcon,
} satisfies Record<string, IconifyIcon>

export type AppIconName = keyof typeof appIconRegistry

type IconBaseProps = Omit<ComponentProps<typeof Icon>, 'icon'>

export interface AppIconProps extends IconBaseProps {
  name: AppIconName
}

export function AppIcon({ name, ...props }: AppIconProps) {
  return <Icon icon={appIconRegistry[name]} {...props} />
}
