// SPDX-License-Identifier: AGPL-3.0-only
export const formatBytes = function (bytes : number, decimals = 2) {
  if (bytes === 0) return '0 Bytes'
  var k = 1024,
    dm = decimals || 2,
    sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'],
    i = Math.floor(Math.log(bytes) / Math.log(k))
  return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i]
}

export const fmtBytes = function (bytes : number, decimals = 2) {
  if (bytes === 0) return [0, 'Bytes']
  var k = 1024,
    dm = decimals || 2,
    sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'],
    i = Math.floor(Math.log(bytes) / Math.log(k))
  return [parseFloat((bytes / Math.pow(k, i)).toFixed(dm)), sizes[i]]
}

export const formatDate = (row : any, column : any) => {
  // 获取单元格数据
  let data = row[column.property]
  if (data === null || data === 0) {
    return ''
  }
  let dt = new Date(data * 1000)
  return (
    dt.getFullYear() +
    '-' +
    (dt.getMonth() + 1) +
    '-' +
    dt.getDate() +
    ' ' +
    dt.getHours() +
    ':' +
    dt.getMinutes() +
    ':' +
    dt.getSeconds()
  )
}
